<p align="center"><img src="campus-tray/assets/icons/connected.png" width="80" alt="SHUNet"></p>

# SHUNet · 上海大学校园网工具

面向已适配 SHU 门户的校园网认证工具，提供 **Windows 原生托盘版**和 **Android 原生精简版**。支持校园网、中国电信、中国联通和中国移动服务选择，实际可用服务以学校门户及账号权限为准。

[下载 Windows 最新版](https://github.com/yanhuigang173/SHUNet/releases/latest) · [Android 安装包](https://github.com/yanhuigang173/SHUNet/releases/tag/android-v0.4.0) · [使用说明](docs/USAGE.md) · [更新记录](CHANGELOG.md) · [反馈问题](https://github.com/yanhuigang173/SHUNet/issues)

这是个人维护的非官方项目，与学校及运营商无隶属关系。认证只适配项目中实现的 SHU 门户，其他学校和同名热点不能据此保证可用。

## 当前版本

| 平台 | 最新源码 | 安装包 | 要求 |
| --- | --- | --- | --- |
| Windows 托盘版 | **v0.14.3** | `SHUNet-v0.14.3.exe` / 便携 ZIP | Windows 10 / 11，x64 |
| Android 精简版 | **v0.4.0** | `SHUNet-Android-v0.4.0-debug.apk` | Android 8.0 / API 26 及以上 |

Windows 版本使用 `v*` 标签，Android 使用 `android-v*` 标签，各自独立更新。Windows 稳定发布标记为 **Latest**；当前 Android 包是调试签名预览包，单独标记为预发布，正式签名包尚未提供。仓库主分支只保留两端最新源码，软件附件放在 Releases。

## 能做什么

| 功能 | Windows | Android |
| --- | --- | --- |
| 账号配置、运营商选择、认证与下线 | ✓ | ✓ |
| 只读查询认证状态 | ✓ | ✓ |
| 系统加密保存密码 | Windows DPAPI | Android Keystore / AES-GCM |
| SHU 常用网址，自定义收藏 | ✓ | ✓ |
| 自动认证、掉线重连、状态检测策略 | ✓ | — |
| Wi-Fi 列表、原生托盘图标、全局快捷键 | ✓ | — |
| 系统手动代理开关、直连 / 代理测速 | ✓ | — |
| IP 信息、共享设备与邻居缓存查看 | ✓ | — |
| 72 小时本机记录、菜单主题、定时电源 | ✓ | — |

Android 需要手动操作，没有常驻后台服务。Windows 的自动策略需要程序保持运行；网页下线保留 Wi-Fi 连接，手动下线会暂停该网络的自动认证。

## 快速使用

**Windows**：下载 EXE 或解压便携 ZIP，退出旧版后运行。程序位于任务栏通知区域，没有常驻主窗口；左键查询状态，右键打开菜单。在“设置 → 账号与密码”填写信息并选择运营商，再点击“一键认证”。首次使用可先保持自动认证和自动重连关闭，确认手动操作正常后按需开启。

**Android**：下载标有 `debug` 的 APK，按系统提示安装。先在系统中连接校园 Wi-Fi，再打开应用保存账号、选择运营商并点击“一键认证”。移动数据可同时开启，门户请求仍绑定物理 Wi-Fi。调试签名包用于体验与验证；升级能否保留数据取决于包名和签名一致，不能用不同签名直接覆盖安装。

验证码、欠费、账号服务未开通等情况需在学校认证网页处理。Windows 上的 ShuWlan-1X 属于 802.1X 网络，请通过系统 Wi-Fi 设置连接，程序不会对它执行网页认证。

## 数据与网络

- 没有广告、统计 SDK 或远程日志。密码仅在用户主动勾选记住后加密保存在本机。
- Windows 配置位于 `%APPDATA%\CampusLinkTray\settings.json`，运行记录位于同目录的 `runtime-logs`。DPAPI 加密密码只可由原电脑的同一 Windows 用户解密。
- Android 配置保存在应用私有目录，禁止云备份和设备迁移；卸载应用会清除本机数据。
- 校园认证访问 `http://10.10.9.9/`，按门户协议加密密码；这并不使整个 HTTP 连接获得 HTTPS 的传输保护。
- Windows 的公网查询使用 ipwho.is，测速使用 Cloudflare；仅在用户打开查询或执行测速时请求。外部服务可看到连接出口 IP；测速会产生真实流量。
- 定时电源计划由 Windows 计划任务执行，退出托盘后仍可生效；需要停用时在定时电源窗口取消计划。

详见 [使用与常见问题](docs/USAGE.md)、[Windows 说明](campus-tray/README.md)、[Android 说明](shunet-android/README.md)。

## 从源码构建

Windows 需要 Rust stable（GNU 工具链）及 MinGW-w64，确保 `cargo`、`gcc`、`windres` 位于 PATH：

```powershell
rustup target add x86_64-pc-windows-gnu
.\scripts\build-windows.ps1
```

输出到 `dist/windows/`。脚本使用临时英文 target 路径，避免 MinGW 处理中文路径出错。运行软件无需安装 Rust、Python 或 .NET。

Android 使用 JDK 17、Android SDK Platform 36、Build-Tools 35.0.0：

```powershell
.\shunet-android\build.ps1 -JavaHome 'C:\path\to\jdk-17' -SdkPath 'C:\path\to\Android\Sdk'
```

也可使用 Android Studio 打开 `shunet-android/`。调试包输出到 `shunet-android/dist/`；正式签名与升级要求见 Android 说明。

## 仓库结构

```text
campus-tray/       Windows 原生托盘源码、资源与 Cargo.lock
shunet-android/    Android 原生源码、Gradle Wrapper 与构建脚本
docs/             使用、历史更新和发布说明
scripts/          Windows 构建、发布打包、GitHub 发布脚本
.github/          问题模板与 PR 模板
```

开发与验证见 [CONTRIBUTING.md](CONTRIBUTING.md)，发布流程见 [docs/RELEASING.md](docs/RELEASING.md)。本仓库不启用自动构建：在本机手动构建、验证后，将安装包上传到 Releases。构建通过不能替代校园 Wi-Fi 下的实机认证验证。

## 许可

项目代码以 [MIT License](LICENSE) 发布，版权归 yanhuigang173。第三方依赖及 Gradle Wrapper 继续适用各自许可，见 [第三方说明](THIRD_PARTY_NOTICES.md)。
