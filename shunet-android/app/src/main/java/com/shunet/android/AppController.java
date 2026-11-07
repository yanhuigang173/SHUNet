package com.shunet.android;

import android.content.Context;
import android.net.ConnectivityManager;
import android.net.LinkProperties;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.NetworkRequest;
import android.os.Handler;
import android.os.Looper;

import java.io.IOException;
import java.time.LocalTime;
import java.time.format.DateTimeFormatter;
import java.util.ArrayDeque;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.function.Consumer;

/** Retained across rotation; one bounded worker and no periodic background work. */
final class AppController implements AutoCloseable {
    enum Action { QUERY, LOGIN, LOGOUT, SAVE }
    static final class UiState {
        Action action;
        PortalClient.State state = PortalClient.State.UNKNOWN;
        String network = "正在读取 Wi-Fi…";
        String title = "认证状态待查询";
        String detail = "连接校园 Wi-Fi 后，点击一键认证或刷新状态。";
        String phase = "";
        String logs = "";
        boolean busy;
        boolean wifi;
        UiState copy() {
            UiState copy = new UiState();
            copy.action = action;
            copy.state = state; copy.network = network; copy.title = title;
            copy.detail = detail; copy.phase = phase; copy.logs = logs;
            copy.busy = busy; copy.wifi = wifi;
            return copy;
        }
    }

    private final Handler main = new Handler(Looper.getMainLooper());
    private final ConnectivityManager connectivity;
    private final CredentialStore credentials;
    private final ThreadPoolExecutor worker;
    private final UiState state = new UiState();
    private final ArrayDeque<String> logs = new ArrayDeque<>();
    private Consumer<UiState> listener;
    private WifiRoute lastRoute;
    private volatile PortalClient activeClient;
    private boolean observing, closed;
    String draftUsername, draftPassword;
    boolean draftRemember;
    Operator draftOperator;

    private final ConnectivityManager.NetworkCallback networkCallback = new ConnectivityManager.NetworkCallback() {
        @Override public void onAvailable(Network network) { scheduleNetworkUpdate(); }
        @Override public void onLost(Network network) { scheduleNetworkUpdate(); }
        @Override public void onCapabilitiesChanged(Network network, NetworkCapabilities caps) { scheduleNetworkUpdate(); }
        @Override public void onLinkPropertiesChanged(Network network, LinkProperties links) { scheduleNetworkUpdate(); }
    };
    private final Runnable networkUpdate = this::refreshLocalNetwork;

    AppController(Context context) {
        Context application = context.getApplicationContext();
        connectivity = application.getSystemService(ConnectivityManager.class);
        credentials = new CredentialStore(application);
        CredentialStore.Saved saved = credentials.load();
        draftUsername = saved.username; draftPassword = saved.password; draftRemember = saved.remember;
        draftOperator = saved.operator;
        worker = new ThreadPoolExecutor(1, 1, 30, TimeUnit.SECONDS,
                new ArrayBlockingQueue<>(1), runnable -> new Thread(runnable, "shunet-portal"));
        worker.allowCoreThreadTimeOut(true);
        log(saved.warning.isEmpty() ? "SHUNet 已启动，等待手动操作。" : saved.warning);
    }

    void setListener(Consumer<UiState> value) { listener = value; emit(); }

    boolean selectOperator(Operator operator) {
        if (closed || state.busy) return false;
        credentials.saveOperator(operator);
        draftOperator = operator;
        // This changes only the next login's service, never the current session.
        return true;
    }

    void startObserving() {
        if (closed) return;
        if (!observing) {
            try {
                NetworkRequest request = new NetworkRequest.Builder()
                        .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                        .removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET).build();
                connectivity.registerNetworkCallback(request, networkCallback);
                observing = true;
            } catch (RuntimeException ignored) { log("网络变化通知暂不可用，可手动刷新连接信息。"); }
        }
        refreshLocalNetwork();
    }

    void stopObserving() {
        if (observing) {
            try { connectivity.unregisterNetworkCallback(networkCallback); } catch (RuntimeException ignored) { }
            observing = false;
        }
        main.removeCallbacks(networkUpdate);
    }

    private void scheduleNetworkUpdate() {
        main.removeCallbacks(networkUpdate);
        main.postDelayed(networkUpdate, 150);
    }

    private void refreshLocalNetwork() {
        if (closed) return;
        WifiRoute current = null;
        String problem = "";
        try { current = WifiRoute.capture(connectivity); }
        catch (IOException | RuntimeException ignored) { problem = "请连接校园 Wi-Fi，等待获取 IPv4 地址。"; }
        boolean changed = lastRoute == null ? current != null : !lastRoute.sameAs(current);
        state.wifi = current != null;
        state.network = current != null ? current.label() : "未发现可用 Wi-Fi";
        if (changed || current == null) {
            PortalClient client = activeClient;
            if (client != null) client.close();
            state.state = PortalClient.State.UNKNOWN;
            state.title = current == null ? "请先连接校园 Wi-Fi" : "认证状态待查询";
            state.detail = current == null ? problem : "连接已变化，点击刷新状态或一键认证。";
        }
        lastRoute = current;
        emit();
    }

    void start(Action action) {
        if (closed || state.busy) return;
        final String username = draftUsername.trim();
        final String password = draftPassword;
        final boolean remember = draftRemember;
        final Operator operator = draftOperator;
        WifiRoute captured = null;
        if (action != Action.SAVE) {
            try { captured = WifiRoute.capture(connectivity); }
            catch (IOException error) { refreshLocalNetwork(); showFailure(error.getMessage()); return; }
            catch (RuntimeException ignored) { showFailure("无法读取手机 Wi-Fi，请在系统设置中检查连接。"); return; }
        }
        final WifiRoute route = captured;
        if (route != null) {
            // Record the exact route this job owns before callbacks arrive.
            // A delayed notification for the same new Wi-Fi must not cancel it.
            if (!route.sameAs(lastRoute)) {
                state.state = PortalClient.State.UNKNOWN;
                state.title = "认证状态待查询";
                state.detail = "当前 Wi-Fi 已变化，正在重新确认认证状态。";
            }
            lastRoute = route;
            state.wifi = true;
            state.network = route.label();
        }
        state.busy = true;
        state.action = action;
        state.phase = action == Action.SAVE ? "正在保存账号设置…" : "正在准备校园网络操作…";
        emit();
        worker.execute(() -> {
            PortalClient client = null;
            PortalClient.Result result = null;
            String failure = null;
            try {
                if (action == Action.SAVE || action == Action.LOGIN) credentials.save(username, password, remember, operator);
                if (action != Action.SAVE) {
                    client = new PortalClient(route, phase -> main.post(() -> {
                        if (!closed && state.busy) { state.phase = phase; emit(); }
                    }));
                    activeClient = client;
                    result = switch (action) {
                        case QUERY -> client.query();
                        case LOGIN -> client.login(username, password, operator);
                        case LOGOUT -> client.logout();
                        default -> null;
                    };
                    client.assertCurrent();
                }
            } catch (IOException error) {
                failure = error.getMessage();
            } catch (RuntimeException ignored) {
                failure = "本次操作未能完成，请刷新状态后重试。";
            } finally {
                if (client != null) client.close();
                activeClient = null;
            }
            PortalClient.Result completed = result;
            String error = failure;
            main.post(() -> finish(action, route, completed, error));
        });
    }

    private void finish(Action action, WifiRoute route, PortalClient.Result result, String failure) {
        if (closed) return;
        state.busy = false;
        state.phase = "";
        if (action != Action.SAVE) {
            try { route.assertCurrent(); }
            catch (IOException ignored) { failure = "Wi-Fi 连接或 IP 已变化，请重新查询当前网络。"; }
        }
        if (failure != null) {
            if (action == Action.SAVE) {
                state.phase = failure;
                log(failure);
                emit();
            } else {
                showFailure(failure);
            }
            return;
        }
        if (action == Action.SAVE) {
            state.phase = draftRemember && !draftPassword.isEmpty()
                    ? "账号设置已保存，密码已在本机加密。" : "账号设置已保存，未在本机保存密码。";
            log(state.phase);
        } else if (result != null) {
            lastRoute = route;
            state.wifi = true;
            state.network = route.label();
            state.state = result.state;
            state.title = result.title;
            state.detail = result.detail;
            log(result.title + " · " + result.detail);
        }
        emit();
    }

    private void showFailure(String detail) {
        state.state = PortalClient.State.UNKNOWN;
        state.title = "暂未确认网络状态";
        state.detail = detail == null ? "操作未完成，请稍后重试。" : detail;
        log(state.detail);
        emit();
    }

    private void log(String text) {
        while (logs.size() >= 10) logs.removeFirst();
        logs.addLast(LocalTime.now().format(DateTimeFormatter.ofPattern("HH:mm:ss")) + "  " + text);
        state.logs = String.join("\n", logs);
    }

    private void emit() { if (!closed && listener != null) listener.accept(state.copy()); }

    @Override public void close() {
        closed = true;
        listener = null;
        stopObserving();
        PortalClient client = activeClient;
        if (client != null) client.close();
        worker.shutdownNow();
        main.removeCallbacksAndMessages(null);
        draftPassword = "";
    }
}
