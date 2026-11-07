# 发布到 GitHub

目标仓库：`yanhuigang173/SHUNet`，公开，MIT 许可。首次发布采用 Windows `v0.14.3` 和 Android `android-v0.4.0` 两个独立标签。Windows 发布标为 Latest，Android 调试签名包标为预发布，不覆盖稳定版 Latest。

## 1. 验证与打包

构建 Windows 并运行默认测试：

```powershell
.\scripts\build-windows.ps1
```

构建 Android：

```powershell
.\shunet-android\build.ps1 -JavaHome 'C:\path\to\jdk-17' -SdkPath 'C:\path\to\Android\Sdk'
```

准备下载附件：

```powershell
.\scripts\prepare-release.ps1 -AndroidApk '.\shunet-android\dist\SHUNet-Android-v0.4.0-debug.apk'
```

也可用 `-WindowsExe` 指定已验证的现有 Windows 包。脚本检查 EXE 产品版本与源码一致，要求 Android 文件名与源码版本一致，并生成便携 ZIP、原文件和 SHA-256 清单。Android 包内容和签名还应使用 SDK 的 `aapt` / `apksigner` 检查，文件名本身不能证明签名或版本正确。

## 2. Git 提交与身份验证

只在本独立目录操作，不推送上级目录的仓库。先检查 `git status` 和待提交文件；构建输出已被忽略。

```powershell
git add .
git commit -m "Prepare SHUNet Windows 0.14.3 and Android 0.4.0 release"
gh auth login --hostname github.com --git-protocol https --web
gh auth setup-git
```

Git 负责提交、标签和推送；GitHub CLI 只负责创建 GitHub 仓库与 Release、上传下载附件。不要将访问令牌写进脚本、提交记录、远程 URL 或聊天。

## 3. 首次创建仓库并发布

登录账号应为 `yanhuigang173`。在确认提交和发布文件后运行：

```powershell
.\scripts\publish-github.ps1
```

脚本创建新仓库、推送 main 和两个标签，先发布 Android 预览，再发布 Windows 稳定版并设为 Latest。发现目标仓库已有内容时会停止，避免覆盖现有项目。发布部分成功后请检查网页再继续，脚本不会删除旧发布或强制推送。以后版本更新请根据新的版本号同步修改 Release notes 和发布命令。

## 后续版本

主分支持续保留两端最新源码，历史由 Git 提交和平台标签保存，不向源码目录重复复制旧版本。Windows 修改 `campus-tray/Cargo.toml` 并更新锁文件中的自身包版本；Android 修改 `app/build.gradle` 的 `versionName` 与递增的 `versionCode`。

正式 Android 发布需由维护者创建并长期保管发行签名密钥。当前调试签名包不应改名冒充正式版本，签名私钥不得提交仓库。自动构建生成的调试包可能使用不同调试密钥，不能保证覆盖现有安装。

发布后核对仓库首页、main 源码、两个标签、Latest 指向、附件数量与 SHA-256；下载到设备再进行校园网络实机验证。
