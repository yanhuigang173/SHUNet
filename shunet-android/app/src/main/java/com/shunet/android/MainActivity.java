package com.shunet.android;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.content.res.ColorStateList;
import android.graphics.Color;
import android.graphics.Insets;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.graphics.drawable.StateListDrawable;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;
import android.text.Editable;
import android.text.InputFilter;
import android.text.InputType;
import android.text.TextUtils;
import android.text.TextWatcher;
import android.view.Gravity;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.Button;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.ImageButton;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.ScrollView;
import android.widget.Switch;
import android.widget.TextView;
import android.widget.Toast;

/** Compact native screen, with actions before expandable account and history sections. */
public final class MainActivity extends Activity {
    private static final int BLUE = 0xFF1677FF, INK = 0xFF182A43, MUTED = 0xFF78869B;
    private static final int GREEN = 0xFF159663, AMBER = 0xFFB97820, LINE = 0xFFE7EDF5;
    private AppController controller;
    private WebsiteStore websites;
    private EditText username, password;
    private Switch remember;
    private TextView network, title, detail, phase, logs, pill, accountSummary, historySummary, accountNotice, websiteSummary;
    private ProgressBar progress;
    private Button login, logout, save, operatorPicker;
    private ImageButton query, passwordToggle, websiteAdd;
    private ImageView stateMark, accountChevron, historyChevron, websiteChevron;
    private LinearLayout accountBody, historyBody, websiteBody;
    private FrameLayout root;
    private boolean accountExpanded, historyExpanded, websitesExpanded, passwordVisible, collapseAfterSave;
    private AlertDialog websiteEditor;
    private EditText websiteNameDraft, websiteUrlDraft;
    private WebsiteStore.Website editingWebsite;
    private String currentDetail = "";

    @Override protected void onCreate(Bundle state) {
        super.onCreate(state);
        Object retained = getLastNonConfigurationInstance();
        controller = retained instanceof AppController ? (AppController) retained : new AppController(this);
        websites = new WebsiteStore(this);
        accountExpanded = state != null ? state.getBoolean("accountExpanded")
                : controller.draftUsername.trim().isEmpty() || controller.draftPassword.isEmpty();
        historyExpanded = state != null && state.getBoolean("historyExpanded");
        websitesExpanded = state != null && state.getBoolean("websitesExpanded");
        collapseAfterSave = state != null && state.getBoolean("collapseAfterSave");
        buildUi();
        if (state == null && !websites.warning.isEmpty()) Toast.makeText(this, websites.warning, Toast.LENGTH_LONG).show();
        if (state != null && state.getBoolean("websiteEditorOpen")) {
            String originalUrl = state.getString("editingWebsiteUrl", "");
            WebsiteStore.Website previous = null;
            for (WebsiteStore.Website website : websites.entries()) {
                if (website.url.equals(originalUrl)) { previous = website; break; }
            }
            if (originalUrl.isEmpty() || previous != null) showWebsiteEditor(previous,
                    state.getString("websiteNameDraft", ""), state.getString("websiteUrlDraft", ""));
        }
    }

    @Override public Object onRetainNonConfigurationInstance() { return controller; }
    @Override protected void onSaveInstanceState(Bundle state) {
        state.putBoolean("accountExpanded", accountExpanded);
        state.putBoolean("historyExpanded", historyExpanded);
        state.putBoolean("websitesExpanded", websitesExpanded);
        state.putBoolean("collapseAfterSave", collapseAfterSave);
        if (websiteEditor != null && websiteEditor.isShowing()) {
            state.putBoolean("websiteEditorOpen", true);
            state.putString("editingWebsiteUrl", editingWebsite == null ? "" : editingWebsite.url);
            state.putString("websiteNameDraft", websiteNameDraft.getText().toString());
            state.putString("websiteUrlDraft", websiteUrlDraft.getText().toString());
        }
        super.onSaveInstanceState(state);
    }
    @Override protected void onStart() {
        super.onStart();
        controller.setListener(this::render);
        controller.startObserving();
    }
    @Override protected void onStop() {
        setPasswordVisible(false);
        controller.setListener(null);
        controller.stopObserving();
        super.onStop();
    }
    @Override protected void onDestroy() {
        if (websiteEditor != null) websiteEditor.dismiss();
        if (!isChangingConfigurations()) controller.close();
        super.onDestroy();
    }

    private void buildUi() {
        root = new FrameLayout(this);
        root.setBackgroundColor(0xFFF4F7FC);
        root.setFocusableInTouchMode(true);
        root.setOnApplyWindowInsetsListener((view, insets) -> {
            if (Build.VERSION.SDK_INT >= 30) {
                Insets value = insets.getInsets(WindowInsets.Type.systemBars()
                        | WindowInsets.Type.displayCutout() | WindowInsets.Type.ime());
                view.setPadding(value.left, value.top, value.right, value.bottom);
            } else {
                view.setPadding(insets.getSystemWindowInsetLeft(), insets.getSystemWindowInsetTop(),
                        insets.getSystemWindowInsetRight(), insets.getSystemWindowInsetBottom());
            }
            return insets;
        });
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setClipToPadding(false);
        scroll.setVerticalScrollBarEnabled(false);
        int width = Math.min(getResources().getDisplayMetrics().widthPixels, dp(600));
        root.addView(scroll, new FrameLayout.LayoutParams(width, -1, Gravity.CENTER_HORIZONTAL));
        LinearLayout content = column();
        content.setPadding(dp(16), dp(12), dp(16), dp(20));
        scroll.addView(content);

        LinearLayout header = row();
        ImageView logo = new ImageView(this);
        logo.setImageResource(R.drawable.ic_shunet);
        logo.setPadding(dp(7), dp(5), dp(7), dp(5));
        logo.setBackground(shape(0xFFEAF3FF, 0, 14));
        logo.setContentDescription("SHUNet");
        header.addView(logo, new LinearLayout.LayoutParams(dp(44), dp(44)));
        LinearLayout brand = column();
        brand.setPadding(dp(11), 0, dp(4), 0);
        brand.addView(text("SHUNet", 22, INK, true));
        add(brand, text("校园网络助手", 11, MUTED, false), 3);
        header.addView(brand, new LinearLayout.LayoutParams(0, -2, 1));
        ImageButton wifiSettings = iconButton(UiIcon.Kind.WIFI, "打开 Wi-Fi 设置", MUTED, false);
        ImageButton help = iconButton(UiIcon.Kind.HELP, "使用说明", MUTED, false);
        header.addView(wifiSettings, new LinearLayout.LayoutParams(dp(48), dp(48)));
        header.addView(help, new LinearLayout.LayoutParams(dp(48), dp(48)));
        content.addView(header);

        LinearLayout status = card(content, 18);
        status.setBackground(new GradientDrawable(GradientDrawable.Orientation.TL_BR,
                new int[]{Color.WHITE, 0xFFF9FBFF}));
        GradientDrawable hero = (GradientDrawable) status.getBackground();
        hero.setCornerRadius(dp(20)); hero.setStroke(dp(1), 0xFFDFE9F7);
        LinearLayout statusHeader = row();
        TextView caption = text("连接概览", 12, MUTED, true);
        statusHeader.addView(caption, new LinearLayout.LayoutParams(0, -2, 1));
        pill = text("待查询", 11, BLUE, true);
        pill.setGravity(Gravity.CENTER);
        pill.setPadding(dp(10), dp(5), dp(10), dp(5));
        statusHeader.addView(pill);
        query = iconButton(UiIcon.Kind.REFRESH, "刷新认证状态", BLUE, false);
        LinearLayout.LayoutParams refreshSize = new LinearLayout.LayoutParams(dp(48), dp(48));
        refreshSize.setMarginStart(dp(5));
        statusHeader.addView(query, refreshSize);
        status.addView(statusHeader);

        LinearLayout connection = row();
        stateMark = new ImageView(this);
        stateMark.setPadding(dp(11), dp(11), dp(11), dp(11));
        connection.addView(stateMark, new LinearLayout.LayoutParams(dp(50), dp(50)));
        LinearLayout message = column();
        message.setPadding(dp(12), 0, 0, 0);
        title = text("认证状态待查询", 21, INK, true);
        title.setMaxLines(2);
        message.addView(title);
        network = text("正在读取 Wi-Fi…", 12, MUTED, false);
        network.setSingleLine(true);
        network.setEllipsize(TextUtils.TruncateAt.END);
        add(message, network, 6);
        connection.addView(message, new LinearLayout.LayoutParams(0, -2, 1));
        add(status, connection, 9);
        detail = text("", 13, MUTED, false);
        detail.setLineSpacing(dp(2), 1);
        detail.setMaxLines(2);
        detail.setEllipsize(TextUtils.TruncateAt.END);
        detail.setPadding(0, dp(2), 0, dp(2));
        add(status, detail, 13);
        phase = text("", 12, BLUE, false);
        phase.setVisibility(View.GONE);
        add(status, phase, 9);
        progress = new ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal);
        progress.setIndeterminate(true);
        progress.setVisibility(View.GONE);
        progress.setIndeterminateTintList(ColorStateList.valueOf(BLUE));
        progress.setPadding(0, 0, 0, 0);
        LinearLayout.LayoutParams barSize = new LinearLayout.LayoutParams(-1, dp(3));
        barSize.topMargin = dp(8);
        status.addView(progress, barSize);
        View divider = new View(this);
        divider.setBackgroundColor(LINE);
        LinearLayout.LayoutParams dividerSize = new LinearLayout.LayoutParams(-1, dp(1));
        dividerSize.topMargin = dp(16); dividerSize.bottomMargin = dp(14);
        status.addView(divider, dividerSize);

        login = button("一键认证", BLUE, Color.WHITE, 0, UiIcon.Kind.CHECK);
        logout = button("校园网下线", 0xFFFFF3F4, 0xFFBB4B5A, 0xFFF5DFE2, UiIcon.Kind.LOGOUT);
        // Respect system font size. Stack actions when their real labels would
        // not fit a narrow screen, instead of shrinking the user's chosen font.
        float available = width - dp(64);
        float required = Math.max(login.getPaint().measureText("一键认证"),
                logout.getPaint().measureText("校园网下线")) + dp(52);
        LinearLayout actions = available >= required * 2 + dp(10) ? row() : column();
        if (actions.getOrientation() == LinearLayout.HORIZONTAL) {
            LinearLayout.LayoutParams left = new LinearLayout.LayoutParams(0, -2, 1);
            left.setMarginEnd(dp(10));
            actions.addView(login, left);
            actions.addView(logout, new LinearLayout.LayoutParams(0, -2, 1));
        } else {
            actions.addView(login, new LinearLayout.LayoutParams(-1, -2));
            add(actions, logout, 8);
        }
        status.addView(actions);
        TextView hint = text("下线后保留 Wi-Fi 连接", 11, MUTED, false);
        hint.setGravity(Gravity.CENTER);
        add(status, hint, 10);

        LinearLayout account = card(content, 14);
        LinearLayout accountHeader = row();
        accountHeader.setMinimumHeight(dp(48));
        ImageView userMark = sectionMark(UiIcon.Kind.USER, BLUE, 0xFFEDF4FF);
        accountHeader.addView(userMark, new LinearLayout.LayoutParams(dp(36), dp(36)));
        LinearLayout accountLabels = column();
        accountLabels.setPadding(dp(11), 0, dp(4), 0);
        accountLabels.addView(text("校园网账号", 15, INK, true));
        accountSummary = text("", 11, MUTED, false);
        accountSummary.setSingleLine(true); accountSummary.setEllipsize(TextUtils.TruncateAt.END);
        add(accountLabels, accountSummary, 4);
        accountHeader.addView(accountLabels, new LinearLayout.LayoutParams(0, -2, 1));
        accountChevron = chevron();
        accountHeader.addView(accountChevron, new LinearLayout.LayoutParams(dp(28), dp(44)));
        account.addView(accountHeader);
        accountBody = column();
        add(account, accountBody, 0);
        addDivider(accountBody, 12);
        add(accountBody, text("账号", 12, MUTED, false), 14);
        username = input("校园网账号", false);
        username.setText(controller.draftUsername);
        add(accountBody, username, 6);
        add(accountBody, text("密码", 12, MUTED, false), 12);
        FrameLayout passwordField = new FrameLayout(this);
        passwordField.setBackground(inputBackground());
        password = input("校园网密码", true);
        password.setBackground(null);
        password.setPadding(dp(12), dp(12), dp(54), dp(12));
        password.setText(controller.draftPassword);
        password.setOnFocusChangeListener((view, focused) -> passwordField.setBackground(
                shape(focused ? Color.WHITE : 0xFFF7F9FD, focused ? BLUE : 0xFFE2E9F2, 11)));
        passwordField.addView(password, new FrameLayout.LayoutParams(-1, -2, Gravity.CENTER_VERTICAL));
        passwordToggle = iconButton(UiIcon.Kind.EYE, "显示密码", MUTED, false);
        passwordField.addView(passwordToggle, new FrameLayout.LayoutParams(dp(48), dp(48), Gravity.END | Gravity.CENTER_VERTICAL));
        add(accountBody, passwordField, 6);
        add(accountBody, text("运营商", 12, MUTED, false), 12);
        operatorPicker = button(controller.draftOperator.label, 0xFFF7F9FD, INK, 0xFFE2E9F2, null);
        operatorPicker.setGravity(Gravity.START | Gravity.CENTER_VERTICAL);
        operatorPicker.setTypeface(Typeface.DEFAULT);
        operatorPicker.setCompoundDrawablesRelativeWithIntrinsicBounds(null, null,
                icon(UiIcon.Kind.CHEVRON, MUTED, 18), null);
        operatorPicker.setOnClickListener(view -> showOperatorPicker());
        add(accountBody, operatorPicker, 6);
        add(accountBody, text("选择后自动保存，下次认证时生效。", 11, MUTED, false), 6);
        remember = new Switch(this);
        remember.setText("记住密码");
        remember.setTextSize(13); remember.setTextColor(INK);
        remember.setMinimumHeight(dp(48)); remember.setSwitchPadding(dp(12));
        remember.setChecked(controller.draftRemember);
        remember.setThumbTintList(new ColorStateList(
                new int[][]{new int[]{android.R.attr.state_checked}, new int[]{}},
                new int[]{BLUE, 0xFFA9B6C9}));
        remember.setTrackTintList(new ColorStateList(
                new int[][]{new int[]{android.R.attr.state_checked}, new int[]{}},
                new int[]{0xFFBFD8FF, 0xFFE0E6EF}));
        add(accountBody, remember, 8);
        add(accountBody, text("勾选后，保存时在此设备加密存储。", 11, MUTED, false), 0);
        accountNotice = text("", 12, BLUE, false);
        accountNotice.setVisibility(View.GONE);
        add(accountBody, accountNotice, 8);
        save = button("保存账号", 0xFFEDF4FF, BLUE, 0, null);
        add(accountBody, save, 13);
        accountHeader.setBackground(ripple(Color.TRANSPARENT, 0, 12));
        accountHeader.setOnClickListener(view -> setAccountExpanded(!accountExpanded));
        accountHeader.setContentDescription("展开或收起校园网账号设置");
        setAccountExpanded(accountExpanded);

        LinearLayout websiteCard = card(content, 12);
        LinearLayout websiteHeader = row();
        LinearLayout websiteToggle = row();
        websiteToggle.setMinimumHeight(dp(48));
        websiteToggle.addView(sectionMark(UiIcon.Kind.GLOBE, BLUE, 0xFFEDF4FF),
                new LinearLayout.LayoutParams(dp(36), dp(36)));
        LinearLayout websiteLabels = column();
        websiteLabels.setPadding(dp(11), 0, dp(4), 0);
        websiteLabels.addView(text("SHU 常用网址", 15, INK, true));
        websiteSummary = text("", 11, MUTED, false);
        add(websiteLabels, websiteSummary, 4);
        websiteToggle.addView(websiteLabels, new LinearLayout.LayoutParams(0, -2, 1));
        websiteChevron = chevron();
        websiteToggle.addView(websiteChevron, new LinearLayout.LayoutParams(dp(28), dp(44)));
        websiteToggle.setBackground(ripple(Color.TRANSPARENT, 0, 12));
        websiteToggle.setOnClickListener(view -> setWebsitesExpanded(!websitesExpanded));
        websiteToggle.setContentDescription("展开或收起 SHU 常用网址");
        websiteHeader.addView(websiteToggle, new LinearLayout.LayoutParams(0, -2, 1));
        websiteAdd = iconButton(UiIcon.Kind.PLUS, "添加常用网址", BLUE, false);
        websiteAdd.setOnClickListener(view -> showWebsiteEditor(null, "", ""));
        websiteHeader.addView(websiteAdd, new LinearLayout.LayoutParams(dp(48), dp(48)));
        websiteCard.addView(websiteHeader);
        websiteBody = column();
        websiteCard.addView(websiteBody, new LinearLayout.LayoutParams(-1, -2));
        setWebsitesExpanded(websitesExpanded);

        LinearLayout history = card(content, 12);
        LinearLayout historyHeader = row();
        historyHeader.setMinimumHeight(dp(48));
        historyHeader.addView(sectionMark(UiIcon.Kind.HISTORY, MUTED, 0xFFF0F3F8),
                new LinearLayout.LayoutParams(dp(36), dp(36)));
        LinearLayout historyLabels = column();
        historyLabels.setPadding(dp(11), 0, dp(4), 0);
        historyLabels.addView(text("操作记录", 15, INK, true));
        historySummary = text("最近的操作会显示在这里", 11, MUTED, false);
        add(historyLabels, historySummary, 4);
        historyHeader.addView(historyLabels, new LinearLayout.LayoutParams(0, -2, 1));
        historyChevron = chevron();
        historyHeader.addView(historyChevron, new LinearLayout.LayoutParams(dp(28), dp(44)));
        history.addView(historyHeader);
        historyBody = column();
        add(history, historyBody, 0);
        addDivider(historyBody, 12);
        logs = text("", 12, MUTED, false);
        logs.setLineSpacing(dp(5), 1); logs.setTextIsSelectable(true);
        add(historyBody, logs, 12);
        historyHeader.setBackground(ripple(Color.TRANSPARENT, 0, 12));
        historyHeader.setOnClickListener(view -> setHistoryExpanded(!historyExpanded));
        historyHeader.setContentDescription("展开或收起操作记录");
        setHistoryExpanded(historyExpanded);

        TextView footer = text("SHUNet  ·  " + BuildConfig.VERSION_NAME, 11, 0xFF9BA7B7, false);
        footer.setGravity(Gravity.CENTER);
        add(content, footer, 20);
        setContentView(root);
        root.requestFocus();
        root.requestApplyInsets();

        username.addTextChangedListener(watcher(() -> {
            controller.draftUsername = username.getText().toString(); updateAccountSummary();
        }));
        password.addTextChangedListener(watcher(() -> {
            controller.draftPassword = password.getText().toString(); updateAccountSummary();
        }));
        remember.setOnCheckedChangeListener((button, checked) -> controller.draftRemember = checked);
        passwordToggle.setOnClickListener(view -> setPasswordVisible(!passwordVisible));
        login.setOnClickListener(view -> { hideKeyboard(); controller.start(AppController.Action.LOGIN); });
        logout.setOnClickListener(view -> { hideKeyboard(); controller.start(AppController.Action.LOGOUT); });
        query.setOnClickListener(view -> { hideKeyboard(); controller.start(AppController.Action.QUERY); });
        save.setOnClickListener(view -> {
            collapseAfterSave = true;
            hideKeyboard();
            controller.start(AppController.Action.SAVE);
        });
        wifiSettings.setOnClickListener(view -> startActivity(new Intent(Settings.ACTION_WIFI_SETTINGS)));
        help.setOnClickListener(view -> showHelp());
        detail.setOnClickListener(view -> new AlertDialog.Builder(this).setTitle("状态说明")
                .setMessage(currentDetail).setPositiveButton("知道了", null).show());
    }

    private void render(AppController.UiState value) {
        title.setText(value.title); detail.setText(value.detail); network.setText(value.network);
        currentDetail = value.detail;
        detail.setContentDescription(value.detail + "，点击查看完整说明");
        logs.setText(value.logs);
        String[] entries = value.logs.isEmpty() ? new String[0] : value.logs.split("\n");
        String latest = entries.length == 0 ? "" : entries[entries.length - 1];
        historySummary.setText(entries.length == 0 ? "暂无操作记录"
                : entries.length + " 条记录  ·  最近 " + latest.substring(0, Math.min(8, latest.length())));
        boolean saving = value.action == AppController.Action.SAVE;
        boolean networkBusy = value.busy && !saving;
        phase.setText(value.phase);
        phase.setVisibility(!saving && !value.phase.isEmpty() ? View.VISIBLE : View.GONE);
        progress.setVisibility(networkBusy ? View.VISIBLE : View.GONE);
        accountNotice.setText(saving ? value.phase : "");
        accountNotice.setVisibility(saving && !value.phase.isEmpty() ? View.VISIBLE : View.GONE);
        int color = value.state == PortalClient.State.ONLINE ? GREEN
                : value.state == PortalClient.State.OFFLINE ? AMBER : BLUE;
        pill.setText(networkBusy ? (value.action == AppController.Action.QUERY ? "查询中" : "处理中")
                : value.state == PortalClient.State.ONLINE ? "已认证"
                : value.state == PortalClient.State.OFFLINE ? "未认证" : "待查询");
        pill.setTextColor(color);
        pill.setBackground(shape((color & 0x00FFFFFF) | 0x13000000, 0, 20));
        stateMark.setImageDrawable(icon(value.state == PortalClient.State.ONLINE
                ? UiIcon.Kind.CHECK : UiIcon.Kind.WIFI, color, 28));
        stateMark.setBackground(shape((color & 0x00FFFFFF) | 0x13000000, 0, 16));
        stateMark.setContentDescription(value.title);
        login.setEnabled(!value.busy && value.wifi);
        logout.setEnabled(!value.busy && value.wifi);
        query.setEnabled(!value.busy && value.wifi);
        save.setEnabled(!value.busy);
        save.setText(value.busy && saving ? "正在保存…" : "保存账号");
        username.setEnabled(!value.busy); password.setEnabled(!value.busy);
        remember.setEnabled(!value.busy); passwordToggle.setEnabled(!value.busy);
        operatorPicker.setEnabled(!value.busy);
        for (View control : new View[]{login, logout, query, save, passwordToggle, operatorPicker}) {
            control.setAlpha(control.isEnabled() ? 1f : .42f);
        }
        if (collapseAfterSave && !value.busy) {
            collapseAfterSave = false;
            if (saving && value.phase.startsWith("账号设置已保存")) {
                setAccountExpanded(false);
                Toast.makeText(this, "账号设置已保存", Toast.LENGTH_SHORT).show();
            }
        }
        if (!value.busy && (value.title.equals("请填写账号密码")
                || value.title.equals("所选运营商不可用"))) setAccountExpanded(true);
    }

    private void setAccountExpanded(boolean expanded) {
        accountExpanded = expanded;
        accountBody.setVisibility(expanded ? View.VISIBLE : View.GONE);
        accountChevron.setRotation(expanded ? 180 : 0);
        if (!expanded) { setPasswordVisible(false); hideKeyboard(); }
        updateAccountSummary();
    }
    private void setHistoryExpanded(boolean expanded) {
        historyExpanded = expanded;
        historyBody.setVisibility(expanded ? View.VISIBLE : View.GONE);
        historyChevron.setRotation(expanded ? 180 : 0);
    }
    private void setWebsitesExpanded(boolean expanded) {
        websitesExpanded = expanded;
        websiteChevron.setRotation(expanded ? 180 : 0);
        websiteBody.setVisibility(expanded ? View.VISIBLE : View.GONE);
        renderWebsites();
    }
    private void renderWebsites() {
        int count = websites.entries().size();
        websiteSummary.setText(count == 0 ? "暂无网址 · 点击右侧添加"
                : count + " 个网址 · " + (websitesExpanded ? "点击打开，右侧菜单管理" : "点击展开"));
        websiteAdd.setEnabled(count < WebsiteStore.MAX_WEBSITES);
        websiteAdd.setAlpha(websiteAdd.isEnabled() ? 1f : .42f);
        websiteBody.removeAllViews();
        // Inflate rows only while expanded; no favicon downloads or background requests.
        if (!websitesExpanded) return;
        addDivider(websiteBody, 10);
        if (count == 0) {
            add(websiteBody, text("点击右上角的加号，添加常用的校园服务网址。", 13, MUTED, false), 14);
            return;
        }
        for (WebsiteStore.Website website : websites.entries()) {
            LinearLayout item = row();
            LinearLayout open = row();
            open.setMinimumHeight(dp(58));
            open.setPadding(dp(2), dp(9), dp(4), dp(9));
            ImageView openMark = new ImageView(this);
            openMark.setImageDrawable(icon(UiIcon.Kind.OPEN, BLUE, 18));
            openMark.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
            open.addView(openMark, new LinearLayout.LayoutParams(dp(18), dp(18)));
            LinearLayout labels = column();
            labels.setPadding(dp(11), 0, 0, 0);
            TextView name = text(website.name, 14, INK, false);
            name.setMaxLines(2); name.setEllipsize(TextUtils.TruncateAt.END);
            labels.addView(name);
            TextView host = text(Uri.parse(website.url).getHost(), 11, MUTED, false);
            host.setSingleLine(true); host.setEllipsize(TextUtils.TruncateAt.END);
            add(labels, host, 5);
            open.addView(labels, new LinearLayout.LayoutParams(0, -2, 1));
            open.setBackground(ripple(Color.TRANSPARENT, 0, 10));
            open.setContentDescription(website.name + "，" + website.url + "，打开网页");
            open.setOnClickListener(view -> openWebsite(website));
            item.addView(open, new LinearLayout.LayoutParams(0, -2, 1));
            ImageButton more = iconButton(UiIcon.Kind.MORE, "管理：" + website.name, MUTED, false);
            more.setOnClickListener(view -> new AlertDialog.Builder(this).setTitle(website.name)
                    .setItems(new String[]{"编辑网址", "删除网址"}, (dialog, which) -> {
                        if (which == 0) showWebsiteEditor(website, website.name, website.url);
                        else confirmDeleteWebsite(website);
                    }).setNegativeButton("取消", null).show());
            item.addView(more, new LinearLayout.LayoutParams(dp(48), dp(48)));
            add(websiteBody, item, 4);
        }
    }
    private void openWebsite(WebsiteStore.Website website) {
        hideKeyboard();
        setPasswordVisible(false);
        try {
            WebsiteStore.Website checked = WebsiteStore.Website.create(website.name, website.url);
            startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(checked.url)).addCategory(Intent.CATEGORY_BROWSABLE));
        } catch (ActivityNotFoundException | SecurityException ignored) {
            Toast.makeText(this, "无法打开网页，请检查是否已安装并启用浏览器。", Toast.LENGTH_LONG).show();
        } catch (IllegalArgumentException error) {
            Toast.makeText(this, error.getMessage(), Toast.LENGTH_LONG).show();
        }
    }
    private void confirmDeleteWebsite(WebsiteStore.Website website) {
        new AlertDialog.Builder(this).setTitle("删除常用网址")
                .setMessage("删除“" + website.name + "”？\n\n" + website.url)
                .setNegativeButton("取消", null)
                .setPositiveButton("删除", (dialog, which) -> {
                    try {
                        websites.delete(website);
                        renderWebsites();
                        Toast.makeText(this, "网址已删除", Toast.LENGTH_SHORT).show();
                    } catch (RuntimeException ignored) {
                        Toast.makeText(this, "删除失败，请重新打开后重试。", Toast.LENGTH_LONG).show();
                    }
                }).show();
    }
    private void showWebsiteEditor(WebsiteStore.Website previous, String initialName, String initialUrl) {
        if (websiteEditor != null && websiteEditor.isShowing()) return;
        hideKeyboard();
        setPasswordVisible(false);
        editingWebsite = previous;
        LinearLayout form = column();
        form.setPadding(dp(22), dp(14), dp(22), dp(4));
        form.addView(text("名称", 12, MUTED, false));
        websiteNameDraft = input("例如：图书馆", false);
        websiteNameDraft.setFilters(new InputFilter[]{new InputFilter.LengthFilter(WebsiteStore.MAX_NAME)});
        websiteNameDraft.setText(initialName);
        websiteNameDraft.setImeOptions(EditorInfo.IME_ACTION_NEXT);
        add(form, websiteNameDraft, 7);
        add(form, text("网址", 12, MUTED, false), 14);
        websiteUrlDraft = input("https://…", false);
        websiteUrlDraft.setFilters(new InputFilter[]{new InputFilter.LengthFilter(WebsiteStore.MAX_URL)});
        websiteUrlDraft.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI);
        websiteUrlDraft.setImeOptions(EditorInfo.IME_ACTION_DONE);
        websiteUrlDraft.setText(initialUrl);
        add(form, websiteUrlDraft, 7);
        add(form, text("省略协议时补 https://，保存后在本机保留。", 11, MUTED, false), 10);
        TextView errorNotice = text("", 12, 0xFFBB4B5A, false);
        errorNotice.setVisibility(View.GONE);
        errorNotice.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        add(form, errorNotice, 8);
        websiteEditor = new AlertDialog.Builder(this).setTitle(previous == null ? "添加常用网址" : "编辑常用网址")
                .setView(form).setPositiveButton("保存", null).setNegativeButton("取消", null).create();
        AlertDialog dialog = websiteEditor;
        dialog.setOnDismissListener(value -> {
            if (websiteEditor == dialog) {
                websiteEditor = null; websiteNameDraft = null; websiteUrlDraft = null; editingWebsite = null;
            }
        });
        dialog.show();
        dialog.getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE);
        dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(view -> {
            try {
                websites.save(previous, websiteNameDraft.getText().toString(), websiteUrlDraft.getText().toString());
                setWebsitesExpanded(true);
                dialog.dismiss();
                Toast.makeText(this, "常用网址已保存", Toast.LENGTH_SHORT).show();
            } catch (IllegalArgumentException error) {
                errorNotice.setText(error.getMessage()); errorNotice.setVisibility(View.VISIBLE);
            } catch (RuntimeException ignored) {
                errorNotice.setText("网址保存失败，请稍后重试。"); errorNotice.setVisibility(View.VISIBLE);
            }
        });
        websiteUrlDraft.setOnEditorActionListener((view, action, event) -> {
            if (action == EditorInfo.IME_ACTION_DONE) {
                dialog.getButton(AlertDialog.BUTTON_POSITIVE).performClick();
                return true;
            }
            return false;
        });
    }
    private void updateAccountSummary() {
        if (accountSummary == null) return;
        String name = controller.draftUsername.trim();
        String summary = name.isEmpty() ? "尚未配置，点击填写"
                : "账号 ····" + name.substring(Math.max(0, name.length() - 4));
        if (!name.isEmpty() && controller.draftPassword.isEmpty()) summary += " · 密码待填写";
        else if (!accountExpanded && !name.isEmpty()) summary += " · 点击编辑";
        accountSummary.setText(controller.draftOperator.label + " · "
                + (accountExpanded ? "用于校园网络认证" : summary));
        operatorPicker.setText(controller.draftOperator.label);
        operatorPicker.setContentDescription("运营商：" + controller.draftOperator.label + "，点击更改");
    }
    private void showOperatorPicker() {
        hideKeyboard();
        Operator[] choices = Operator.values();
        String[] labels = new String[choices.length];
        for (int i = 0; i < choices.length; i++) labels[i] = choices[i].label;
        new AlertDialog.Builder(this).setTitle("选择运营商")
                .setSingleChoiceItems(labels, controller.draftOperator.ordinal(), (dialog, which) -> {
                    Operator selected = choices[which];
                    if (controller.selectOperator(selected)) {
                        updateAccountSummary();
                        Toast.makeText(this, "已保存：" + selected.label + "，下次认证时生效", Toast.LENGTH_SHORT).show();
                    }
                    dialog.dismiss();
                })
                .setNegativeButton("取消", null).show();
    }
    private void setPasswordVisible(boolean visible) {
        if (password == null || passwordToggle == null) return;
        passwordVisible = visible;
        int cursor = password.getSelectionStart();
        password.setInputType(InputType.TYPE_CLASS_TEXT | (visible
                ? InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD : InputType.TYPE_TEXT_VARIATION_PASSWORD));
        password.setTypeface(Typeface.DEFAULT);
        password.setSelection(Math.max(0, Math.min(cursor, password.length())));
        passwordToggle.setImageDrawable(icon(visible ? UiIcon.Kind.EYE_OFF : UiIcon.Kind.EYE, MUTED, 20));
        passwordToggle.setContentDescription(visible ? "隐藏密码" : "显示密码");
        passwordToggle.setTooltipText(visible ? "隐藏密码" : "显示密码");
    }
    private void hideKeyboard() {
        if (root == null) return;
        root.requestFocus();
        InputMethodManager manager = getSystemService(InputMethodManager.class);
        if (manager != null && root.getWindowToken() != null) {
            manager.hideSoftInputFromWindow(root.getWindowToken(), 0);
        }
    }
    private void showHelp() {
        new AlertDialog.Builder(this).setTitle("SHUNet 使用说明")
                .setMessage("1. 在手机系统设置连接校园 Wi-Fi。\n\n"
                        + "2. 展开校园网账号，填写账号密码，选择运营商并按需保存账号，然后点击一键认证。已经在线时会跳过重复登录。\n\n"
                        + "3. 连接卡片右上角可刷新状态；校园网下线只退出认证，保留 Wi-Fi。\n\n"
                        + "运营商默认为校园网，也可选择中国电信、中国联通或中国移动，选择后自动保存。更改运营商不会中断已在线的会话，在下次认证时生效。所选服务必须已由学校为该账号开通，应用不会自动换用其他运营商。\n\n"
                        + "账号已填写时默认收起。密码可点输入框内的眼睛查看，收起设置或离开应用后会自动隐藏。\n\n"
                        + "勾选记住密码并保存后，密码在此设备加密存储；取消勾选后保存可删除已保存密码。\n\n"
                        + "状态说明可点击查看全文，操作记录可展开查看。查询期间保留原状态，结果返回后更新。\n\n"
                        + "SHU 常用网址预置 9 个校园服务入口。展开后点击网址打开网页，右侧菜单可编辑或删除，右上角加号可添加，最多保存 64 个。更改自动保存；部分服务需要校园网或学校统一登录。\n\n"
                        + "门户请求使用当前 Wi-Fi；VPN 锁定或系统限制可能阻止访问。")
                .setPositiveButton("知道了", null).show();
    }

    private EditText input(String hint, boolean secret) {
        EditText input = new EditText(this);
        input.setSingleLine(true); input.setTextSize(15); input.setTextColor(INK);
        input.setHintTextColor(0xFF9CA8B8); input.setHint(hint);
        input.setInputType(InputType.TYPE_CLASS_TEXT | (secret
                ? InputType.TYPE_TEXT_VARIATION_PASSWORD : InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS));
        input.setTypeface(Typeface.DEFAULT);
        input.setFilters(new InputFilter[]{new InputFilter.LengthFilter(128)});
        input.setPadding(dp(12), dp(12), dp(12), dp(12));
        input.setMinimumHeight(dp(48));
        input.setBackground(inputBackground());
        input.setSaveEnabled(false);
        input.setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO);
        return input;
    }
    private StateListDrawable inputBackground() {
        StateListDrawable background = new StateListDrawable();
        background.addState(new int[]{android.R.attr.state_focused}, shape(Color.WHITE, BLUE, 11));
        background.addState(new int[]{}, shape(0xFFF7F9FD, 0xFFE2E9F2, 11));
        return background;
    }
    private LinearLayout card(LinearLayout parent, int margin) {
        LinearLayout card = column();
        card.setPadding(dp(16), dp(12), dp(16), dp(16));
        card.setBackground(shape(Color.WHITE, LINE, 18));
        add(parent, card, margin);
        return card;
    }
    private ImageView sectionMark(UiIcon.Kind kind, int color, int background) {
        ImageView mark = new ImageView(this);
        mark.setImageDrawable(icon(kind, color, 20));
        mark.setPadding(dp(8), dp(8), dp(8), dp(8));
        mark.setBackground(shape(background, 0, 12));
        mark.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        return mark;
    }
    private ImageView chevron() {
        ImageView chevron = new ImageView(this);
        chevron.setImageDrawable(icon(UiIcon.Kind.CHEVRON, MUTED, 18));
        chevron.setPadding(dp(5), dp(10), dp(5), dp(10));
        chevron.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        return chevron;
    }
    private ImageButton iconButton(UiIcon.Kind kind, String label, int color, boolean filled) {
        ImageButton button = new ImageButton(this);
        button.setImageDrawable(icon(kind, color, 20));
        button.setPadding(dp(12), dp(12), dp(12), dp(12));
        button.setBackground(ripple(filled ? 0xFFEDF4FF : Color.TRANSPARENT, 0, 13));
        button.setContentDescription(label); button.setTooltipText(label);
        button.setMinimumWidth(dp(44)); button.setMinimumHeight(dp(44));
        return button;
    }
    private Button button(String value, int fill, int color, int border, UiIcon.Kind symbol) {
        Button button = new Button(this);
        button.setText(value); button.setTextSize(14); button.setAllCaps(false);
        button.setTextColor(color); button.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        button.setMinHeight(dp(50)); button.setMinimumHeight(dp(50));
        button.setMinWidth(0); button.setMinimumWidth(0);
        button.setPadding(dp(12), dp(12), dp(12), dp(12));
        button.setStateListAnimator(null);
        button.setBackground(ripple(fill, border, 12));
        if (symbol != null) {
            button.setCompoundDrawablesRelativeWithIntrinsicBounds(icon(symbol, color, 18), null, null, null);
            button.setCompoundDrawablePadding(dp(6));
        }
        return button;
    }
    private UiIcon icon(UiIcon.Kind kind, int color, int size) { return new UiIcon(kind, color, dp(size)); }
    private LinearLayout column() {
        LinearLayout layout = new LinearLayout(this); layout.setOrientation(LinearLayout.VERTICAL); return layout;
    }
    private LinearLayout row() {
        LinearLayout layout = new LinearLayout(this); layout.setGravity(Gravity.CENTER_VERTICAL);
        layout.setBaselineAligned(false); return layout;
    }
    private TextView text(String value, int size, int color, boolean bold) {
        TextView view = new TextView(this);
        view.setText(value); view.setTextSize(size); view.setTextColor(color);
        view.setIncludeFontPadding(false);
        if (bold) view.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        return view;
    }
    private void addDivider(LinearLayout parent, int margin) {
        View divider = new View(this);
        divider.setBackgroundColor(LINE);
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, dp(1));
        params.topMargin = dp(margin);
        parent.addView(divider, params);
    }
    private GradientDrawable shape(int fill, int border, int radius) {
        GradientDrawable drawable = new GradientDrawable();
        drawable.setColor(fill); drawable.setCornerRadius(dp(radius));
        if (border != 0) drawable.setStroke(dp(1), border);
        return drawable;
    }
    private RippleDrawable ripple(int fill, int border, int radius) {
        return new RippleDrawable(ColorStateList.valueOf(0x181677FF), shape(fill, border, radius), null);
    }
    private void add(LinearLayout parent, View child, int margin) {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2);
        params.topMargin = dp(margin); parent.addView(child, params);
    }
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }
    private static TextWatcher watcher(Runnable changed) {
        return new TextWatcher() {
            public void beforeTextChanged(CharSequence value, int start, int count, int after) { }
            public void onTextChanged(CharSequence value, int start, int before, int count) { changed.run(); }
            public void afterTextChanged(Editable value) { }
        };
    }
}
