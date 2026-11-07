package com.shunet.android;

import android.os.SystemClock;
import android.text.Html;
import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.CookieManager;
import java.net.CookiePolicy;
import java.net.HttpURLConnection;
import java.net.Proxy;
import java.net.URI;
import java.nio.charset.Charset;
import java.nio.charset.StandardCharsets;
import java.util.Collections;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.function.Consumer;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/** SHU ePortal protocol ported from campus-tray/src/network.rs. */
final class PortalClient implements AutoCloseable {
    enum State { ONLINE, OFFLINE, UNKNOWN }

    static final class Result {
        final State state;
        final String title, detail;
        Result(State state, String title, String detail) {
            this.state = state; this.title = title; this.detail = detail;
        }
    }

    private static final URI GATEWAY = URI.create("http://10.10.9.9/");
    private final WifiRoute route;
    private final Consumer<String> progress;
    private final CookieManager cookies = new CookieManager(null, CookiePolicy.ACCEPT_ORIGINAL_SERVER);
    private final long deadline = SystemClock.elapsedRealtime() + 60_000;
    private volatile HttpURLConnection activeConnection;
    private volatile boolean cancelled;

    PortalClient(WifiRoute route, Consumer<String> progress) {
        this.route = route; this.progress = progress;
    }

    Result query() throws IOException {
        progress.accept("正在通过校园 Wi-Fi 查询状态…");
        Page page = discover();
        Session session = onlineSession(page, true);
        if (session.state == State.ONLINE) return online("门户已确认当前手机认证在线。");
        if (session.state == State.OFFLINE) {
            validateLoginPage(page);
            return offline("当前未认证", "门户已确认未认证，可以点击一键认证。");
        }
        return unknown("状态待确认", "门户正在同步认证状态，请稍后刷新；本次未提交账号密码。");
    }

    Result login(String username, String password, Operator operator) throws IOException {
        progress.accept("正在检查是否已经认证…");
        Page page = discover();
        Session session = onlineSession(page, true);
        if (session.state == State.ONLINE) return online("校园网已经认证，无需重复登录。");
        if (session.state != State.OFFLINE) {
            return unknown("暂未提交认证", "门户尚未明确确认未认证，请稍后重新查询。");
        }
        validateLoginPage(page);
        if (username.trim().isEmpty() || password.isEmpty()) {
            return offline("请填写账号密码", "填写校园网账号密码后，再点击一键认证。");
        }
        if (username.length() > 128 || password.length() > 128) {
            throw new IOException("账号和密码长度不能超过 128 个字符。");
        }

        String query = page.uri.getRawQuery();
        progress.accept("正在读取门户认证参数…");
        Reply infoReply = request(endpoint(page, "pageInfo"), "POST", "queryString=" + encode(query), page.uri);
        requireOk(infoReply, "读取认证参数");
        JSONObject info = jsonOrEmpty(infoReply.body);
        String encryptValue = field(info, page.html, "passwordEncrypt");
        if (encryptValue.isEmpty()) throw new IOException("门户未提供密码加密方式，尚未提交账号密码。");
        boolean encrypted = encryptValue.equalsIgnoreCase("true") || encryptValue.equals("1");
        if (!encrypted && !encryptValue.equalsIgnoreCase("false") && !encryptValue.equals("0")) {
            throw new IOException("无法识别门户密码加密方式，尚未提交账号密码。");
        }
        String service = "";
        try {
            Reply services = request(URI.create(endpoint(page, "getServices") + "&queryString=" + encode(query)),
                    "POST", "", page.uri);
            if (services.status == 200) service = chooseService(services.body, operator);
        } catch (IOException ignored) { assertCurrent(); }
        if (service.isEmpty()) service = serviceFromHtml(page.html, operator);
        if (service.isEmpty()) return offline("所选运营商不可用", "未能从门户取得“" + operator.label
                + "”服务，请确认运营商选择或刷新后重试。本次未提交账号密码，也未改用其他运营商。");

        String portalPassword = password;
        if (encrypted) {
            portalPassword = PortalRsa.encrypt(password + ">" + parameter(page.uri, "mac"),
                    field(info, page.html, "publicKeyExponent"), field(info, page.html, "publicKeyModulus"));
        }
        // The original AuthInterFace.js double-encodes every login value. Keep
        // location.search raw until this point; decoding its pairs breaks tokens.
        String body = "userId=" + twice(username.trim()) + "&password=" + twice(portalPassword)
                + "&service=" + twice(service) + "&queryString=" + twice(query)
                + "&operatorPwd=&operatorUserId=&validcode=&passwordEncrypt=" + twice(encrypted ? "true" : "false");
        assertCurrent();
        Session latest = onlineSession(page, false);
        if (latest.state == State.ONLINE) return online("检测到已经认证，已跳过重复登录。");
        if (latest.state != State.OFFLINE) return unknown("暂未提交认证", "门户状态发生变化，请先重新查询。");

        progress.accept("正在提交“" + operator.label + "”认证…");
        Reply loginReply = null;
        try {
            // Exactly one application-level credential POST, including when a
            // response is lost. Confirmation below only sends read requests.
            loginReply = request(endpoint(page, "login"), "POST", body, page.uri);
        } catch (IOException ignored) { assertCurrent(); }
        progress.accept("正在确认认证结果…");
        Session confirmed = confirm(false);
        if (confirmed != null && confirmed.state == State.ONLINE) return online("校园网认证成功，已确认手机当前会话在线。");
        if (loginReply != null) {
            JSONObject reply = jsonOrEmpty(loginReply.body);
            String code = reply.optString("result", reply.optString("code", ""));
            if (!code.isEmpty() && !accepted(code)) {
                return offline("认证未成功", friendlyRejection(
                        reply.optString("message", reply.optString("msg", "")), operator));
            }
        }
        return unknown("认证结果待确认", "认证请求可能已送达，但尚未确认在线。请刷新状态；不会自动重复提交密码。");
    }

    Result logout() throws IOException {
        progress.accept("正在读取当前手机的在线会话…");
        Page page = discover();
        Session session = onlineSession(page, true);
        if (session.state == State.OFFLINE) {
            validateLoginPage(page);
            return offline("当前已下线", "校园网已经处于未认证状态，Wi-Fi 连接保留。");
        }
        if (session.state != State.ONLINE || session.userIndex.isEmpty()) {
            return unknown("暂未提交下线", "未取得当前手机的有效在线会话，请先刷新状态。");
        }
        assertCurrent();
        progress.accept("正在提交校园网下线…");
        try {
            request(endpoint(page, "logout"), "POST", "userIndex=" + encode(session.userIndex), page.uri);
        } catch (IOException ignored) { assertCurrent(); }
        progress.accept("正在确认下线结果…");
        Session confirmed = confirm(true);
        if (confirmed != null && confirmed.state == State.OFFLINE) {
            return offline("校园网已下线", "已确认门户返回未认证状态，Wi-Fi 连接保持不变。");
        }
        return unknown("下线结果待确认", "下线请求可能已送达，但尚未确认结果。请刷新状态，不会自动重复下线。");
    }

    void assertCurrent() throws IOException {
        if (cancelled || Thread.currentThread().isInterrupted()) throw new IOException("本次操作已取消。");
        if (SystemClock.elapsedRealtime() >= deadline) throw new IOException("操作超时，请检查校园 Wi-Fi 后重试。");
        route.assertCurrent();
    }

    @Override public void close() {
        cancelled = true;
        HttpURLConnection connection = activeConnection;
        if (connection != null) connection.disconnect();
    }

    private Session confirm(boolean wantOffline) throws IOException {
        for (int attempt = 0; attempt < 3; attempt++) {
            if (attempt > 0) pause(600L * attempt);
            try {
                Page fresh = discover();
                Session session = onlineSession(fresh, false);
                if (wantOffline && session.state == State.OFFLINE) validateLoginPage(fresh);
                if (session.state == (wantOffline ? State.OFFLINE : State.ONLINE)) return session;
            } catch (IOException ignored) { assertCurrent(); }
        }
        return null;
    }

    private Page discover() throws IOException {
        URI current = GATEWAY;
        Set<String> visited = new HashSet<>();
        for (int hop = 0; hop < 10; hop++) {
            if (!visited.add(current.toString())) throw new IOException("校园门户出现循环跳转，请重新连接 Wi-Fi。");
            Reply reply = request(current, "GET", null, null);
            if (reply.status >= 300 && reply.status < 400) {
                if (reply.location == null || reply.location.isEmpty()) throw new IOException("门户未提供有效跳转地址。");
                try { current = current.resolve(reply.location); }
                catch (IllegalArgumentException ignored) { throw new IOException("门户跳转地址无效。"); }
                continue;
            }
            requireOk(reply, "打开认证页面");
            if (isPortal(current)) return new Page(current, reply.body);
            Matcher script = Pattern.compile("(?is)(?:window\\.|document\\.)?location(?:\\.href)?\\s*=\\s*(['\"])(.*?)\\1").matcher(reply.body);
            boolean found = false;
            while (script.find()) {
                try {
                    URI next = current.resolve(script.group(2).replace("\\/", "/").replace("&amp;", "&"));
                    if (isPortal(next)) { current = next; found = true; break; }
                } catch (IllegalArgumentException ignored) { }
            }
            if (!found) throw new IOException("未识别到 SHU 校园认证页，请确认连接的是校园 Wi-Fi。");
        }
        throw new IOException("校园门户跳转次数过多，请稍后重试。");
    }

    private Session onlineSession(Page page, boolean retryWait) throws IOException {
        Session session = null;
        for (int attempt = 0; attempt < (retryWait ? 3 : 1); attempt++) {
            if (attempt > 0) pause(500L * attempt);
            String index = parameter(page.uri, "userIndex");
            Reply reply = request(endpoint(page, "getOnlineUserInfo"), "POST", "userIndex=" + encode(index), page.uri);
            requireOk(reply, "查询在线状态");
            JSONObject object;
            try { object = new JSONObject(stripBom(reply.body)); }
            catch (Exception ignored) { throw new IOException("门户返回的状态无法解析，认证情况未知。"); }
            String result = object.optString("result", "");
            String returned = object.optString("userIndex", "");
            if (result.equals("success") && (!index.isEmpty() || !returned.isEmpty())
                    && (index.isEmpty() || returned.isEmpty() || index.equals(returned))) {
                String ip = object.optString("userIp", "").trim();
                if (!ip.isEmpty() && !route.addresses.contains(ip)) {
                    throw new IOException("门户在线会话与当前 Wi-Fi 地址不一致，尚未执行操作。");
                }
                return new Session(State.ONLINE, returned.isEmpty() ? index : returned);
            }
            if (result.equals("fail")) return new Session(State.OFFLINE, "");
            session = new Session(State.UNKNOWN, "");
            if (!result.equals("wait")) break;
        }
        return session;
    }

    private void validateLoginPage(Page page) throws IOException {
        if (!isPortal(page.uri) || !page.uri.getPath().toLowerCase(Locale.ROOT).endsWith("/index.jsp")) {
            throw new IOException("尚未取得真实登录页，已停止提交，请刷新校园网络状态。");
        }
        for (String name : new String[]{"wlanuserip", "wlanacname", "ssid", "nasip", "mac", "t"}) {
            if (parameter(page.uri, name).isEmpty()) throw new IOException("门户缺少当前 Wi-Fi 的认证参数，尚未提交账号密码。");
        }
        String ip = parameter(page.uri, "wlanuserip").trim();
        if (ip.matches("[0-9a-fA-F]{32}")) return; // Opaque gateway token; never replace it with a local IP.
        if (!ip.matches("(?:[0-9]{1,3}\\.){3}[0-9]{1,3}") || !route.addresses.contains(ip)) {
            throw new IOException("门户地址参数与当前 Wi-Fi 不匹配，尚未提交账号密码。");
        }
    }

    private Reply request(URI uri, String method, String body, URI referer) throws IOException {
        assertCurrent();
        validateDestination(uri, !method.equals("GET"));
        HttpURLConnection connection = null;
        try {
            connection = (HttpURLConnection) route.network.openConnection(uri.toURL(), Proxy.NO_PROXY);
            activeConnection = connection;
            assertCurrent();
            int timeout = (int) Math.max(1, Math.min(6000, deadline - SystemClock.elapsedRealtime()));
            connection.setConnectTimeout(timeout);
            connection.setReadTimeout(timeout);
            connection.setInstanceFollowRedirects(false);
            connection.setUseCaches(false);
            connection.setRequestMethod(method);
            connection.setRequestProperty("User-Agent", "Mozilla/5.0 (Linux; Android) AppleWebKit/537.36 Chrome/132.0 Mobile Safari/537.36 SHUNet/0.1");
            connection.setRequestProperty("Cache-Control", "no-cache, no-store");
            connection.setRequestProperty("Pragma", "no-cache");
            connection.setRequestProperty("Accept-Encoding", "identity");
            for (Map.Entry<String, List<String>> entry : cookies.get(uri, Collections.emptyMap()).entrySet()) {
                for (String value : entry.getValue()) connection.addRequestProperty(entry.getKey(), value);
            }
            if (referer != null) connection.setRequestProperty("Referer", referer.toString());
            if (body != null) {
                connection.setDoOutput(true);
                connection.setRequestProperty("Origin", "http://10.10.9.9");
                connection.setRequestProperty("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8");
                byte[] bytes = body.getBytes(StandardCharsets.UTF_8);
                connection.setFixedLengthStreamingMode(bytes.length);
                try (OutputStream stream = connection.getOutputStream()) { assertCurrent(); stream.write(bytes); }
            }
            int status = connection.getResponseCode();
            cookies.put(uri, connection.getHeaderFields());
            if (!method.equals("GET") && status >= 300 && status < 400) {
                throw new IOException("门户接口返回意外跳转，尚未确认结果，请刷新状态。");
            }
            InputStream stream = status >= 400 ? connection.getErrorStream() : connection.getInputStream();
            String text = "";
            if (stream != null) {
                try (InputStream input = stream; ByteArrayOutputStream output = new ByteArrayOutputStream()) {
                    byte[] buffer = new byte[4096];
                    int count;
                    while ((count = input.read(buffer)) != -1) {
                        assertCurrent();
                        if (output.size() + count > 512 * 1024) throw new IOException("门户响应超出大小限制。");
                        output.write(buffer, 0, count);
                    }
                    Charset charset = StandardCharsets.UTF_8;
                    String type = connection.getContentType();
                    if (type != null) {
                        Matcher match = Pattern.compile("(?i)charset=([\\w-]+)").matcher(type);
                        if (match.find()) {
                            try { charset = Charset.forName(match.group(1)); } catch (Exception ignored) { }
                        }
                    }
                    text = new String(output.toByteArray(), charset);
                }
            }
            assertCurrent();
            return new Reply(status, text, connection.getHeaderField("Location"));
        } catch (IOException failure) {
            assertCurrent();
            // Do not show platform exception messages: they can contain URLs,
            // query tokens, cookies or request data.
            if (failure.getMessage() != null && (failure.getMessage().startsWith("门户")
                    || failure.getMessage().startsWith("操作"))) throw failure;
            throw new IOException("无法通过当前 Wi-Fi 完成门户请求，请检查校园网络后重试。");
        } finally {
            if (connection != null) connection.disconnect();
            activeConnection = null;
        }
    }

    private static void validateDestination(URI uri, boolean api) throws IOException {
        String host = uri.getHost();
        if (!"http".equalsIgnoreCase(uri.getScheme()) || uri.getRawUserInfo() != null
                || (uri.getPort() != -1 && uri.getPort() != 80)
                || !("10.10.9.9".equals(host) || (!api && "123.123.123.123".equals(host)))) {
            throw new IOException("门户跳转到了未支持的地址，尚未提交账号密码。");
        }
        if (api && !isPortal(uri)) throw new IOException("门户接口来源不正确，已停止提交。");
    }

    private static boolean isPortal(URI uri) {
        return "http".equalsIgnoreCase(uri.getScheme()) && "10.10.9.9".equals(uri.getHost())
                && uri.getRawUserInfo() == null && (uri.getPort() == -1 || uri.getPort() == 80)
                && uri.getPath() != null && uri.getPath().toLowerCase(Locale.ROOT).startsWith("/eportal/");
    }

    private static URI endpoint(Page page, String method) throws IOException {
        if (!isPortal(page.uri)) throw new IOException("未取得真实校园门户。");
        return page.uri.resolve("InterFace.do?method=" + method);
    }

    private static String parameter(URI uri, String name) throws IOException {
        String raw = uri.getRawQuery();
        if (raw == null) return "";
        try {
            for (String part : raw.split("&")) {
                String[] pair = part.split("=", 2);
                if (java.net.URLDecoder.decode(pair[0], "UTF-8").equalsIgnoreCase(name)) {
                    return pair.length > 1 ? java.net.URLDecoder.decode(pair[1], "UTF-8") : "";
                }
            }
        } catch (IllegalArgumentException ignored) { throw new IOException("门户参数编码无效，尚未执行操作。"); }
        return "";
    }

    private static String encode(String value) {
        StringBuilder result = new StringBuilder();
        for (byte item : value.getBytes(StandardCharsets.UTF_8)) {
            int b = item & 255;
            if ((b >= 'A' && b <= 'Z') || (b >= 'a' && b <= 'z') || (b >= '0' && b <= '9')
                    || "-_.!~*'()".indexOf(b) >= 0) result.append((char) b);
            else result.append('%').append("0123456789ABCDEF".charAt(b >>> 4)).append("0123456789ABCDEF".charAt(b & 15));
        }
        return result.toString();
    }
    private static String twice(String value) { return encode(encode(value)); }
    private static String stripBom(String value) { return value.startsWith("\uFEFF") ? value.substring(1) : value; }
    private static JSONObject jsonOrEmpty(String body) {
        try { return new JSONObject(stripBom(body)); } catch (Exception ignored) { return new JSONObject(); }
    }
    private static String field(JSONObject object, String html, String name) {
        for (JSONObject candidate : new JSONObject[]{object, object.optJSONObject("data"), object.optJSONObject("pageInfo")}) {
            if (candidate != null && candidate.has(name) && !candidate.isNull(name)) {
                String value = candidate.optString(name, "");
                if (!value.isEmpty()) return value;
            }
        }
        Matcher assignment = Pattern.compile("(?is)\\b" + Pattern.quote(name) + "\\s*[:=]\\s*(['\"])(.*?)\\1").matcher(html);
        if (assignment.find()) return assignment.group(2);
        Matcher inputs = Pattern.compile("(?is)<input\\b[^>]*>").matcher(html);
        while (inputs.find()) if (attribute(inputs.group(), "name").equalsIgnoreCase(name)) return attribute(inputs.group(), "value");
        return "";
    }
    private static String attribute(String tag, String name) {
        Matcher match = Pattern.compile("(?is)\\b" + Pattern.quote(name) + "\\s*=\\s*(['\"])(.*?)\\1").matcher(tag);
        return match.find() ? match.group(2) : "";
    }
    private static String serviceFromHtml(String html, Operator operator) {
        Matcher options = Pattern.compile("(?is)<option\\b([^>]*)>([^<]*)").matcher(html);
        while (options.find()) {
            String value = htmlText(attribute(options.group(1), "value"));
            String display = htmlText(options.group(2)).trim();
            if (!value.isEmpty() && operator.matches(value, display)) return value;
        }
        // The SHU page also renders DIV choices instead of SELECT options.
        // Read only the two quoted literals; never evaluate portal JavaScript.
        Matcher tags = Pattern.compile("(?is)<[a-z][^>]*>").matcher(html);
        Pattern serviceCall = Pattern.compile("^\\s*selectService\\(\\s*(['\"])([^'\"]*)\\1\\s*,\\s*(['\"])([^'\"]*)\\3\\s*\\)\\s*;?\\s*$");
        while (tags.find()) {
            String tag = tags.group();
            String id = attribute(tag, "id");
            if (id.equals("net_access_type")) {
                String value = htmlText(attribute(tag, "value"));
                if (!value.isEmpty() && operator.matches(value, "")) return value;
            }
            if (!id.startsWith("bch_service_")) continue;
            Matcher call = serviceCall.matcher(htmlText(attribute(tag, "onclick")));
            if (call.matches()) {
                String value = call.group(2), display = call.group(4);
                if (!value.isEmpty() && operator.matches(value, display)) return value;
            }
        }
        String value = field(new JSONObject(), html, "net_access_type");
        return operator.matches(value, "") ? value : "";
    }
    private static String htmlText(String value) {
        return Html.fromHtml(value, Html.FROM_HTML_MODE_LEGACY).toString();
    }
    private static String chooseService(String body, Operator operator) {
        try {
            Object data = new JSONObject(stripBom(body)).opt("serviceJson");
            JSONArray list = data instanceof JSONArray ? (JSONArray) data : new JSONArray(String.valueOf(data));
            for (int i = 0; i < list.length(); i++) {
                JSONObject item = list.optJSONObject(i);
                if (item == null) continue;
                String value = item.optString("serviceName", item.optString("id", ""));
                String display = item.optString("serviceShowName", "");
                if (!value.isEmpty() && operator.matches(value, display)) return value;
            }
            return "";
        } catch (Exception ignored) { return ""; }
    }
    private static boolean accepted(String result) {
        return result.equalsIgnoreCase("success") || result.equalsIgnoreCase("ok") || result.equals("1");
    }
    private static String friendlyRejection(String message, Operator operator) {
        if (message.contains("未绑定") || message.contains("未开通") || message.contains("未授权")) {
            return "门户提示账号未绑定、未开通或未获授权使用“" + operator.label
                    + "”，请检查账号与运营商选择。当前选择已保留。";
        }
        if (message.contains("密码") || message.toLowerCase(Locale.ROOT).contains("password")) return "门户提示账号或密码有误，请检查后重试。";
        if (message.contains("验证码")) return "门户要求验证码，请先在系统认证网页完成登录。";
        if (message.contains("欠费") || message.contains("余额")) return "门户提示账号费用异常，请检查校园网账户。";
        if (message.contains("未注册")) return "门户提示认证设备未注册，请重新连接校园 Wi-Fi 并刷新状态。";
        return "门户未接受认证，请检查账号密码，或在系统认证网页查看原因。";
    }
    private static void requireOk(Reply reply, String operation) throws IOException {
        if (reply.status < 200 || reply.status >= 300) throw new IOException("门户" + operation + "返回 HTTP " + reply.status + "，尚未确认结果。");
    }
    private void pause(long millis) throws IOException {
        assertCurrent();
        try { Thread.sleep(millis); } catch (InterruptedException ignored) { Thread.currentThread().interrupt(); }
        assertCurrent();
    }
    private static Result online(String message) { return new Result(State.ONLINE, "校园网已认证", message); }
    private static Result offline(String title, String detail) { return new Result(State.OFFLINE, title, detail); }
    private static Result unknown(String title, String detail) { return new Result(State.UNKNOWN, title, detail); }

    private static final class Page {
        final URI uri; final String html;
        Page(URI uri, String html) { this.uri = uri; this.html = html; }
    }
    private static final class Reply {
        final int status; final String body, location;
        Reply(int status, String body, String location) { this.status = status; this.body = body; this.location = location; }
    }
    private static final class Session {
        final State state; final String userIndex;
        Session(State state, String userIndex) { this.state = state; this.userIndex = userIndex; }
    }
}
