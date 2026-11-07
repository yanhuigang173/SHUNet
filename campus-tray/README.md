# SHUNet Windows 托盘版

当前版本 **v0.14.3**。Rust + Windows 原生 API，没有常驻主窗口、控制台、Slint 或 WebView。支持 Windows 10 / 11 x64；Windows 其他版本未在本次发布中验证。

[下载稳定版](https://github.com/yanhuigang173/SHUNet/releases/latest) · [完整使用说明](../docs/USAGE.md) · [历史更新](../docs/CHANGELOG-windows.md)

## 使用

退出旧版，再运行 `SHUNet-v0.14.3.exe` 或便携包中的 `SHUNet.exe`。通知区域可能将新图标收进隐藏区域。左键立即查询一次，右键打开菜单。

1. 通过 Windows 连接校园 Wi-Fi。
2. “设置 → 账号与密码”配置校园账号，按需记住密码；选择已开通的运营商。
3. 点击“一键认证”，程序先查当前会话，已在线时不重复提交密码。
4. 按需启用自动认证、自动重连、登录启动或全局快捷键。

ShuWlan-1X 通过 Windows 配置 802.1X 账号，软件不对它进行门户检测、网页登录或网页下线。

## 功能入口

| 入口 | 功能 |
| --- | --- |
| 一键认证 / 校园网下线 | 查询后提交，按门户返回证据确认结果；下线保留 Wi-Fi |
| 自动认证 / 自动重连 | 首次未认证登录与意外掉线恢复分别控制 |
| Wi-Fi 网络 | 附近网络、刷新、连接、断开及系统 Wi-Fi 设置 |
| SHU 大常用网址 | 内置学校服务，可新增、编辑、删除 |
| 校园网工具 | 认证页、直连 / 代理测速、IP 信息、共享设备、定时电源 |
| 运行状态与记录 | 状态详情、今日重连、最近 72 小时日志、筛选与复制 |
| 设置 | 账号、运营商、快捷键、菜单主题、检测策略、自动与启动、导入 / 导出 |

图标颜色：灰色无可识别连接；蓝色普通网络、待确认或暂停；绿色经门户确认在线；橙色认证 / 下线处理中；红色明确掉线、拒绝或检测异常。查询角标只表示正在处理，不能据此判断在线。

默认快捷键为 `Ctrl+Shift+U` 一键认证、`Ctrl+Shift+S` 查询状态，可在设置中修改或清除。

## 配置与边界

配置位于 `%APPDATA%\CampusLinkTray\settings.json`，密码使用当前用户的 Windows DPAPI 加密；未勾选记住时仅存在于本次进程内。首次启动可能沿用原桌面版配置，之后独立保存。导出配置里的加密密码只可在原电脑的同一用户下恢复。

状态检测、首次自动认证和掉线自动重连分别控制。未知状态、超时和门户 `wait` 不直接等同于下线。手动下线暂停当前 SSID 的自动策略，程序重启和网络切换后仍保留；手动认证或“恢复当前网络的自动策略”可解除。

网络请求绑定物理 Wi-Fi，网络身份变化会使旧操作失效。Wi-Fi 物理断开时等待 Windows 恢复连接，不主动切换到其他网络。代理开关只切换当前用户的手动代理，不启动代理服务。

公网查询使用 ipwho.is，测速使用 Cloudflare，均为按需请求。测速每次下载 / 上传载荷上限约为 32 MiB / 8 MiB，另有协议开销；数值属于到测试服务的 HTTP 单连接估计。共享设备来自 Windows 热点和邻居缓存，不进行整段校园网扫描，缓存不能证明设备此刻在线。

定时电源使用本机 Windows 计划任务，退出托盘后仍有效；停用请在对应窗口取消。只能尝试从设备支持的睡眠 / 休眠状态唤醒，不能保证从完全关机状态开机。

## 构建与测试

在仓库根目录，确保 Rust GNU 工具链及 MinGW-w64 的 `gcc`、`windres` 位于 PATH：

```powershell
.\scripts\build-windows.ps1
```

也可在本目录手动构建：

```powershell
$trayTarget = Join-Path $env:TEMP 'shunet-build-target'
cargo test --locked --target x86_64-pc-windows-gnu --target-dir $trayTarget
cargo build --locked --release --target x86_64-pc-windows-gnu --target-dir $trayTarget
```

默认测试跳过标记为 `ignored` 的实网认证、原生窗口和计划任务测试。不要用 `--include-ignored` 在自己的当前校园会话上运行全部测试。常规构建不依赖 Python；仅重新生成图标时使用 `tools/generate_icons.py` 与 Pillow。

源码目录可单独构建，资源包含 DPI manifest、图标、原生对话框和定时电源执行脚本。界面设计见 [UI_GUIDE.md](UI_GUIDE.md)，[布局图](assets/ui-preview.svg)为设计示意，并非运行截图。
