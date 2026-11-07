package com.shunet.android;

import android.content.Context;
import android.content.SharedPreferences;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.net.URI;
import java.net.URISyntaxException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Locale;

/** Independent bookmark preferences; account and encrypted password data are untouched. */
final class WebsiteStore {
    static final int MAX_WEBSITES = 64, MAX_NAME = 64, MAX_URL = 2048;
    private final SharedPreferences preferences;
    private List<Website> entries;
    final String warning;

    static final class Website {
        final String name, url;
        private Website(String name, String url) { this.name = name; this.url = url; }

        static Website create(String name, String address) {
            name = name.trim(); address = address.trim();
            if (name.isEmpty() || name.codePointCount(0, name.length()) > MAX_NAME
                    || name.codePoints().anyMatch(Character::isISOControl)) {
                throw new IllegalArgumentException("名称不能为空，最多 64 个字，且不能包含控制字符。");
            }
            if (address.isEmpty() || address.length() > MAX_URL || address.codePoints().anyMatch(
                    c -> Character.isISOControl(c) || Character.isWhitespace(c) || Character.isSpaceChar(c))) {
                throw new IllegalArgumentException("请输入有效网址，最多 2048 个字符，不能包含空白字符。");
            }
            if (!address.contains("://")) address = "https://" + address;
            try {
                URI parsed = new URI(address).normalize();
                String scheme = parsed.getScheme();
                if (scheme == null || (!scheme.equalsIgnoreCase("https") && !scheme.equalsIgnoreCase("http"))
                        || parsed.getHost() == null || parsed.getHost().isEmpty()) {
                    throw new IllegalArgumentException("请输入有效的 http:// 或 https:// 网页地址。");
                }
                if (parsed.getRawUserInfo() != null) {
                    throw new IllegalArgumentException("网址中不能包含账号密码，请在浏览器中登录。");
                }
                int port = parsed.getPort();
                if (port == 0 || port > 65535 || (parsed.getRawAuthority().endsWith(":") && port == -1)) {
                    throw new IllegalArgumentException("网址端口无效。");
                }
                scheme = scheme.toLowerCase(Locale.ROOT);
                StringBuilder url = new StringBuilder(scheme).append("://")
                        .append(parsed.getHost().toLowerCase(Locale.ROOT));
                if (port != -1 && !(scheme.equals("https") && port == 443)
                        && !(scheme.equals("http") && port == 80)) url.append(':').append(port);
                String path = parsed.getRawPath();
                url.append(path == null || path.isEmpty() ? "/" : path);
                if (parsed.getRawQuery() != null) url.append('?').append(parsed.getRawQuery());
                if (parsed.getRawFragment() != null) url.append('#').append(parsed.getRawFragment());
                String normalized = new URI(url.toString()).toASCIIString();
                if (normalized.length() > MAX_URL) throw new IllegalArgumentException("网址过长，最多 2048 个字符。");
                return new Website(name, normalized);
            } catch (URISyntaxException ignored) {
                throw new IllegalArgumentException("网址格式不正确，请检查域名、端口和特殊字符。");
            }
        }
    }

    WebsiteStore(Context context) {
        preferences = context.getApplicationContext().getSharedPreferences("websites", Context.MODE_PRIVATE);
        List<Website> loaded;
        String problem = "";
        try {
            String stored = preferences.getString("entries", null);
            if (stored == null) loaded = defaults();
            else {
                if (stored.length() > 600_000) throw new JSONException("Too large");
                JSONArray array = new JSONArray(stored);
                loaded = new ArrayList<>();
                for (int i = 0; i < array.length() && loaded.size() < MAX_WEBSITES; i++) {
                    try {
                        JSONObject entry = array.getJSONObject(i);
                        Website website = Website.create(entry.getString("name"), entry.getString("url"));
                        if (loaded.stream().noneMatch(old -> old.url.equals(website.url))) loaded.add(website);
                    } catch (JSONException | IllegalArgumentException ignored) { }
                }
                if (loaded.size() != array.length()) problem = "已跳过无效、重复或超出数量限制的常用网址。";
            }
        } catch (JSONException | RuntimeException ignored) {
            loaded = defaults();
            problem = "常用网址配置无法读取，已显示默认网址。";
        }
        entries = Collections.unmodifiableList(loaded);
        warning = problem;
    }

    List<Website> entries() { return entries; }

    void save(Website previous, String name, String address) {
        Website website = Website.create(name, address);
        List<Website> next = new ArrayList<>(entries);
        int index = previous == null ? -1 : next.indexOf(previous);
        if (previous != null && index == -1) throw new IllegalArgumentException("网址已变化，请重新打开后编辑。");
        if (previous == null && next.size() >= MAX_WEBSITES) throw new IllegalArgumentException("最多保存 64 个网址。");
        for (int i = 0; i < next.size(); i++) {
            if (i != index && next.get(i).url.equals(website.url)) throw new IllegalArgumentException("这个网址已经在收藏中。");
        }
        if (index == -1) next.add(website); else next.set(index, website);
        persist(next);
    }

    void delete(Website website) {
        List<Website> next = new ArrayList<>(entries);
        if (!next.remove(website)) throw new IllegalArgumentException("网址已变化，请重新打开后删除。");
        persist(next);
    }

    private void persist(List<Website> next) {
        JSONArray array = new JSONArray();
        try {
            for (Website website : next) array.put(new JSONObject().put("name", website.name).put("url", website.url));
        } catch (JSONException ignored) { throw new IllegalStateException("网址保存失败，请稍后重试。"); }
        preferences.edit().putString("entries", array.toString()).apply();
        // A saved empty list must stay empty after a restart.
        entries = Collections.unmodifiableList(next);
    }

    private static List<Website> defaults() {
        String[][] defaults = {
                {"上海大学迎新系统", "https://yxxt.shu.edu.cn/"},
                {"上海大学研究生招生信息网", "https://yjszs.shu.edu.cn/"},
                {"上海大学迎新网站", "https://yingxin.shu.edu.cn/"},
                {"上海大学邮件系统", "https://newmail.shu.edu.cn/"},
                {"上海大学个人信息门户（一网通办）", "https://eportal.shu.edu.cn/"},
                {"研究生综合服务平台", "https://yjs.shu.edu.cn/"},
                {"研究生选课系统（本研一体化）", "https://byxk.shu.edu.cn/"},
                {"本硕博全生命周期统一管理平台", "https://xuegong.shu.edu.cn/xsfw/sys/xggzptapp/index.do"},
                {"上海大学机电工程与自动化学院", "https://auto.shu.edu.cn/"}
        };
        List<Website> websites = new ArrayList<>();
        for (String[] entry : defaults) websites.add(Website.create(entry[0], entry[1]));
        return websites;
    }
}
