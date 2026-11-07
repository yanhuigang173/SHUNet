package com.shunet.android;

import android.net.ConnectivityManager;
import android.net.LinkAddress;
import android.net.LinkProperties;
import android.net.Network;
import android.net.NetworkCapabilities;

import java.io.IOException;
import java.net.Inet4Address;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/** A specific physical Wi-Fi network, including its current IPv4 identity. */
final class WifiRoute {
    final Network network;
    final List<String> addresses;
    final String fingerprint;
    private final ConnectivityManager connectivity;

    private WifiRoute(ConnectivityManager connectivity, Network network, LinkProperties links)
            throws IOException {
        this.connectivity = connectivity;
        this.network = network;
        addresses = ipv4(links);
        if (addresses.isEmpty()) throw new IOException("Wi-Fi 尚未分配 IPv4 地址，请稍后重试。");
        fingerprint = identity(links);
    }

    static WifiRoute capture(ConnectivityManager connectivity) throws IOException {
        List<Network> candidates = new ArrayList<>();
        for (Network network : connectivity.getAllNetworks()) {
            if (isWifi(connectivity.getNetworkCapabilities(network))) candidates.add(network);
        }
        if (candidates.isEmpty()) throw new IOException("请先在系统设置中连接校园 Wi-Fi。");
        Network selected = connectivity.getActiveNetwork();
        if (!candidates.contains(selected)) {
            if (candidates.size() != 1) {
                throw new IOException("检测到多个 Wi-Fi 连接，请保留校园 Wi-Fi 后重试。");
            }
            selected = candidates.get(0);
        }
        LinkProperties links = connectivity.getLinkProperties(selected);
        if (links == null) throw new IOException("无法读取当前 Wi-Fi 信息，请重新连接后重试。");
        return new WifiRoute(connectivity, selected, links);
    }

    void assertCurrent() throws IOException {
        LinkProperties links = connectivity.getLinkProperties(network);
        if (!isWifi(connectivity.getNetworkCapabilities(network)) || links == null
                || !fingerprint.equals(identity(links))) {
            throw new IOException("Wi-Fi 连接或 IP 已变化，本次操作已停止，请重新查询。");
        }
    }

    boolean sameAs(WifiRoute other) {
        return other != null && network.equals(other.network) && fingerprint.equals(other.fingerprint);
    }

    String label() { return "Wi-Fi 已连接 · " + addresses.get(0); }

    private static boolean isWifi(NetworkCapabilities caps) {
        // Do not require VALIDATED or INTERNET: a captive Wi-Fi can be unvalidated
        // while Android uses cellular data as the default network.
        return caps != null && caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)
                && !caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN);
    }

    private static List<String> ipv4(LinkProperties links) {
        List<String> values = new ArrayList<>();
        for (LinkAddress link : links.getLinkAddresses()) {
            if (link.getAddress() instanceof Inet4Address && !link.getAddress().isLinkLocalAddress()
                    && !link.getAddress().isLoopbackAddress()) {
                values.add(link.getAddress().getHostAddress());
            }
        }
        Collections.sort(values);
        return values;
    }

    private static String identity(LinkProperties links) {
        return links.getInterfaceName() + "|" + String.join(",", ipv4(links));
    }
}
