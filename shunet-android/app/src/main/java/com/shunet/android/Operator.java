package com.shunet.android;

/** Stable configuration values and service names shared with the Windows tray app. */
enum Operator {
    CAMPUS("campus", "校园网", "shu"),
    TELECOM("telecom", "中国电信", "中国电信"),
    UNICOM("unicom", "中国联通", "中国联通"),
    MOBILE("mobile", "中国移动", "中国移动");

    final String key, label;
    private final String serviceName;

    Operator(String key, String label, String serviceName) {
        this.key = key; this.label = label; this.serviceName = serviceName;
    }

    static Operator fromKey(String key) {
        for (Operator operator : values()) if (operator.key.equals(key)) return operator;
        // Existing installations have no operator field and keep campus service.
        return CAMPUS;
    }

    boolean matches(String value, String display) {
        return serviceName.equals(value) || label.equals(value) || label.equals(display);
    }
}
