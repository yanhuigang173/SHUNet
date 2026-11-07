# SHUNet Android

独立的 Android 原生精简版，项目目录为 `shunet-android`。Java 17 + Android 原生控件，支持 **Android 8.0 / API 26 及以上**，目标 API 36。没有额外 UI / 网络运行库，界面使用 SHUNet 标志与蓝白卡片样式。

## SHU 常用网址（v0.4.0）

首页新增可展开的“SHU 常用网址”，默认收起。预置入口沿用托盘版：迎新系统、研究生招生信息网、迎新网站、邮件系统、一网通办、研究生综合服务平台、研究生选课系统（本研一体化）、本硕博全生命周期统一管理平台、机电工程与自动化学院，共 9 个。

- 点击名称在系统浏览器打开；名称下显示域名，右侧菜单可编辑或删除。删除前确认。
- 点击卡片右上角加号可添加网址。名称最多 64 个字、网址最多 2048 个字符，最多保留 64 个入口。
- 省略协议时补 `https://`；支持 HTTP / HTTPS，拒绝空白、无效地址及含账号密码的网址；保存前检查重复地址。
- 新增、编辑和删除自动保存到本机独立的网址配置，不影响账号密码及运营商选择。主动删除全部入口后，空列表仍会保留。
- 展开状态及编辑框中的未保存内容在屏幕旋转后恢复。收起时释放网址行视图，不下载图标或自动访问网页。

部分校园服务需要校园网或学校统一登录。打开网址本身不会执行校园网认证或下线，访问由系统浏览器处理。

## 运营商选择（v0.3.0）

- 展开“校园网账号”后可选择 **校园网、中国电信、中国联通、中国移动**，与托盘版一致。首次安装或从旧版升级时默认为校园网。
- 选择立即保存到本机配置，重启后保留；收起账号面板时，摘要显示当前选择。更改运营商不会保存尚未保存的账号密码编辑。
- 已在线会话继续使用当前连接，新选择在下次认证时生效。应用不会因为更改选项自动下线或重新认证。
- 登录时匹配门户实际返回的服务值。所选服务不可用时不提交密码，也不自动换用默认或其他运营商。校园网使用门户的 `shu` 服务。
- 账号必须已开通对应服务。门户提示未开通、未绑定或未授权时会说明原因，并保留当前选择，方便调整。

## 界面更新（v0.2.0）

- 页头改为紧凑标志和标题，Wi-Fi 设置与帮助放在右上角，点击图标即可打开，长按可查看说明。
- 连接概览集中展示认证状态、Wi-Fi / IP 和简短说明；一键认证、校园网下线放进同一卡片，右上角为刷新状态。说明可点击查看全文。
- 已填好账号密码时默认收起账号设置，摘要只显示账号末尾；缺少信息时默认展开。保存成功后自动收起。
- 输入框、记住密码开关、框内密码显隐按钮统一尺寸。收起账号设置或离开应用时重新隐藏密码。
- 操作记录默认收起，展示条数和最近操作时间，展开可查看完整记录并选择文字。
- 按屏幕实际宽度和系统字体大小安排按钮；窄屏或大字体时上下排列，保留系统字体缩放。页面处理状态栏、导航栏和键盘安全边距。

界面规范见 [UI_GUIDE.md](UI_GUIDE.md)。

## 已实现

- **一键认证**：先检查当前手机是否已经认证，已在线则跳过登录。明确未认证、取得真实门户参数后才提交一次账号密码，再只读确认结果。
- **校园网下线**：查询本机当前在线会话，再提交一次下线，重新发现门户并确认未认证。下线保留 Wi-Fi 连接。
- **刷新状态**：只查询，不提交账号密码。查询进度独立显示，期间保留上次认证状态，结果返回后更新。
- **运营商选择**：默认校园网，另有电信、联通、移动；选择自动保存，认证前从真实门户匹配对应服务。
- **SHU 常用网址**：预置 9 个校园服务入口，可新增、编辑和删除，自动保存在本机配置中。
- **账号设置**：保存账号；勾选记住密码后，使用 Android Keystore 的 AES-GCM 密钥加密保存。取消勾选并点击“保存账号设置”会删除已保存密码。
- **网络变化保护**：所有门户请求使用选中的物理 Wi-Fi `Network`，忽略手动 HTTP 代理。Wi-Fi 网络句柄、接口或 IPv4 地址变化时停止旧操作。移动数据可用不代表校园网已认证。
- 浅色原生界面、操作阶段提示、最近 10 条内存记录、系统 Wi-Fi 设置入口、内置使用说明。屏幕旋转保留当前任务和内存中的输入，不重新提交认证。

本版需要手动操作，没有自动认证、自动重连、开机启动、测速、桌面托盘或常驻后台服务。应用打开时只读取本机连接信息，不自动提交账号密码。

## 安装与使用

当前发布的是**调试签名预览包**：`SHUNet-Android-v0.4.0-debug.apk`，下载见 [Android 预发布](https://github.com/yanhuigang173/SHUNet/releases/tag/android-v0.4.0)。本地构建输出位于 `dist/`。覆盖升级需要包名与签名一致；不同构建环境的调试签名可能不同，不能保证覆盖现有安装。尚未提供正式签名版本。

1. 将 APK 发到手机，按 Android 提示允许当前文件来源安装并安装。
2. 在手机系统设置中连接 SHU / CMCC 校园 Wi-Fi。尚未认证时，系统可能提示该网络无法访问互联网，请保留 Wi-Fi 连接。
3. 打开 SHUNet，展开“校园网账号”，填写账号密码，选择运营商（默认校园网），按需勾选记住密码并保存，点击“一键认证”。
4. 需要下线时点击“校园网下线”。需要核对当前认证时点击连接概览卡片右上角的刷新图标。
5. 展开“SHU 常用网址”访问校园服务，点击右上角加号添加网址，或通过每项右侧菜单管理已保存的网址。

可同时保留移动数据。认证和下线仍绑定 Wi-Fi，不能通过移动数据认证校园网。处于非校园 Wi-Fi、缺少真实门户参数、门户状态未知时不会发送密码。锁定模式 VPN 或手机系统策略可能阻止访问物理 Wi-Fi；遇到门户不可达时请先检查这些设置。

“已提交 / 接受”不直接显示成功，必须有在线接口的本机会话证据。响应丢失时只做确认，不自动补发登录或下线。失败时可手动刷新状态后重试；验证码、账号欠费、强制修改密码等需要在学校的系统认证网页处理。

## 从源码构建

推荐使用 Android Studio 打开本文件夹，使用 JDK 17 或其自带的兼容 JDK。在 SDK Manager 安装：

- Android SDK Platform 36
- Android SDK Build-Tools 35.0.0
- Android SDK Platform-Tools（安装 / 连接手机时使用）

首次同步需要访问 Google Maven、Maven Central 和 Gradle 分发地址。项目包含 Gradle Wrapper 8.13，分发包锁定 SHA-256；Android Gradle Plugin 使用 8.13.2。

Windows PowerShell：

```powershell
.\build.ps1 -JavaHome 'C:\Program Files\Android\Android Studio\jbr' `
    -SdkPath "$env:LOCALAPPDATA\Android\Sdk"
```

也可配置 `JAVA_HOME`、`ANDROID_HOME` 后执行 `./gradlew.bat :app:assembleDebug`，或在 Linux / macOS 执行 `sh ./gradlew :app:assembleDebug`。`local.properties.example` 提供 SDK 路径示例，实际的 `local.properties` 不纳入版本控制。

`build.ps1` 将 APK 和 SHA-256 文件复制到 `dist/`；依赖已缓存时可加 `-Offline`，也可用 `-GradleExecutable` 指定现有 Gradle。调试 APK 可直接安装，正式发行请使用 Android Studio 的签名构建功能创建并妥善保存自己的发行密钥。升级安装须保持应用包名与签名一致；本机 Keystore 密钥不能从 Windows 配置导入，也不会随备份导出。

## 协议与实现

认证依据同目录下的 Windows 托盘版 `../campus-tray/src/network.rs` 迁移：

1. 通过 Wi-Fi 访问 `http://10.10.9.9/`，跟随真实 HTTP / JavaScript 字面量跳转。
2. 只有 `10.10.9.9/eportal/` 可以接收门户 API POST；`123.123.123.123` 仅支持无凭据的引导 GET，不对其调用认证接口。
3. `getOnlineUserInfo` 返回明确的在线 / 未认证证据，核对 `userIndex` 和响应中的本机 IPv4。`wait`、解析异常、超时均表示未知。
4. 登录页必须包含 `wlanuserip / wlanacname / ssid / nasip / mac / t`。`wlanuserip` 的 32 位十六进制值作为不透明网关参数保留，明文 IPv4 则核对当前 Wi-Fi 地址。
5. 从 `pageInfo / getServices` 或真实 HTML 读取加密与服务配置，按用户选择匹配 `serviceName / serviceShowName`、`option` 或 `selectService` 字面量中的实际值，不采用其他运营商或门户默认项。密码按原 RSAUtils 的 UTF-16 反转、低字节打包、大整数模幂和四位十六进制分组生成，登录字段与完整原始查询串按网页规则双重 URI 编码。
6. 提交前再次确认会话与 Wi-Fi 身份；同一操作使用独立 Cookie 会话，有请求超时、总时长及响应大小限制。

未内置任何真实账号、密码、会话索引、MAC 或认证参数。没有广告、统计 SDK、远程日志或全局系统代理修改。仅声明 Internet 和网络状态权限；不扫描附近 Wi-Fi、不读取 SSID，不申请定位权限。密码不写入日志或 Activity 状态快照，配置禁止云备份和设备迁移。

主要代码：

| 文件 | 职责 |
| --- | --- |
| `MainActivity.java` | 原生界面、输入与进度、旋转恢复 |
| `UiIcon.java` | 统一线性图标，不依赖图标字体 |
| `AppController.java` | 单个有界后台任务、网络变化监听、状态和记录 |
| `WifiRoute.java` | 选择物理 Wi-Fi 并核对网络身份 |
| `PortalClient.java` | 门户发现、状态查询、认证和下线 |
| `PortalRsa.java` | 门户特定的 RSA 编码 |
| `Operator.java` | 运营商枚举、持久化键与门户服务匹配 |
| `CredentialStore.java` | Android Keystore 加密保存与运营商配置 |
| `WebsiteStore.java` | 默认校园服务、网址校验与独立本机配置 |

平台行为依据 Android 官方文档：[Network.openConnection](https://developer.android.com/reference/android/net/Network)、[网络安全配置](https://developer.android.com/privacy-and-security/security-config)、[Android Keystore](https://developer.android.com/privacy-and-security/keystore)、[AGP 8.13 兼容表](https://developer.android.com/build/releases/agp-8-13-0-release-notes)。

## 验证范围

APK 构建与手机校园网认证是两个独立步骤。没有连接到校园 Wi-Fi 的 Android 实机时，无法由 Windows 的成功认证推断 Android 端同样成功；首次请在手机校园 Wi-Fi 下核对登录、已经在线时重复点击、下线，以及移动数据同时开启时的结果。
