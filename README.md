# AGY Switch

Antigravity 桌面端（2.0）多账号与 GCP 项目一键切换工具，基于 Tauri 2 构建。

在本地加密保存多份登录档案，一键完成账号或 GCP 项目切换；全程仅读写本机凭据，不联网、不上传任何数据。

## 功能特性

- **一键切换账号 / 项目**：自动关闭 Antigravity、将当前账号刷新后的最新 Token 回存至原档案、写入目标凭据并重启应用。
- **无损换号（登录新账号）**：仅清除本机登录状态并重启至登录页，不调用服务端注销接口，避免已保存档案中的 Token 被吊销。
- **防丢防覆盖保护**：
  - 当前登录尚未保存为档案时，自动拦截切换与换号操作。
  - 同名档案仅允许更新同一邮箱与 GCP 项目，防止误覆盖其他账号。
  - 检测到启用 ADC（`enable_adc`）自动登录时，拦截无 ADC 的冲突切换。
- **本地加密存储**：档案使用 Windows DPAPI 加密，绑定当前 Windows 登录账户，跨机无法解密。
- **截图隐私模式**：点击右上角 **隐藏账号** 可将邮箱、档案名与 GCP 项目 ID 替换为模糊占位符，悬停单行可临时查看。

## 使用说明

### 1. 保存当前账号

1. 在 Antigravity 中正常完成登录（个人账号，或选好 GCP 项目 / License）。
2. 打开 **AGY Switch**，点击底部 **保存当前**，输入档案名称并回车保存。

### 2. 添加新账号

1. 点击底部 **登录新账号**：工具会自动回存当前账号最新 Token、清除本机凭据并重启 Antigravity 至登录页。
2. 在 Antigravity 中登录另一个账号。
3. 回到 **AGY Switch** 点击 **刷新**，确认识别到新账号后，点击 **保存当前** 另存为新档案。

### 3. 日常切换

在档案列表中点击目标账号右侧的开关，等待 Antigravity 自动重启即完成切换。

> **注意事项**
> - **切勿在 Antigravity 内点击「退出登录」**：客户端自带的退出会向服务器注销会话，导致已保存档案中的 Token 失效。换号请始终使用本工具的 **登录新账号**。
> - **切换前检查运行状态**：切换过程会关闭并重启 Antigravity，请先确认没有正在执行的 Agent 任务。
> - **会话有效性**：「当前」标记基于本机凭据比对，不代表服务端会话一定有效；若凭据已过期或被撤销，在 Antigravity 中重新登录并保存即可更新档案。
> - **ADC 模式**：「设置」中的 *保存时包含 ADC 文件* 仅在使用 `gcloud` 应用默认凭据（`application_default_credentials.json`）登录时需要开启，个人账号无需勾选。

## 数据存储与路径

| 用途 | 路径 / 位置 | 说明 |
| :--- | :--- | :--- |
| 账号档案 | `%LOCALAPPDATA%\agy-switch\profiles\` | Windows DPAPI 加密存储 |
| 本机凭据 | Windows 凭据管理器 `gemini:antigravity` | 用户名固定为 `antigravity` |
| 降级凭据文件 | `%USERPROFILE%\.gemini\jetski-standalone-oauth-token` | 切换或换号时自动同步清理 |
| 启动错误日志 | `%LOCALAPPDATA%\agy-switch\startup.log` | 仅记录启动诊断信息，不含账号 Token |
| WebView2 缓存 | `%LOCALAPPDATA%\agy-switch\webview-user` | 管理员模式使用 `webview-admin` |

卸载时直接删除 `agy-switch.exe` 与 `%LOCALAPPDATA%\agy-switch` 目录即可，不影响 Antigravity 本身。

## 构建与发布

### 本地编译与安装

```powershell
# 1. 生成应用图标（首次构建或修改图标后执行）
node scripts/gen-icon.mjs

# 2. 编译 Release 版本
cargo build --release --manifest-path src-tauri/Cargo.toml

# 3. 在普通用户 PowerShell 中发布并启动（请先关闭已运行的 AGY Switch）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/publish.ps1 -Launch
```

发布后的可执行文件位于：`%LOCALAPPDATA%\Programs\agy-switch\agy-switch.exe`。

### 打包分发版 ZIP

在普通用户 PowerShell 中执行以下命令，可在清理本机路径信息后生成便携分发包 `dist\AGY-Switch-<版本>-win-x64.zip`（内含 `agy-switch.exe` 与 `使用说明.txt`）：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1
```

### 构建说明与故障排查

- **完整性级别（Mandatory Integrity Control）**：若在低权限沙箱中执行编译，产物会带有 `Low Mandatory Level` 标签，直接运行将无法写入用户目录（或触发 `0x800700AA` 错误）。因此必须通过普通用户 PowerShell 运行 `scripts/publish.ps1` 或 `scripts/package.ps1` 重建正常权限的可执行文件。
- **启动诊断**：若无法启动，请查看 `%LOCALAPPDATA%\agy-switch\startup.log`。状态检查失败时会保留可读取的账号档案，并显示具体错误。

### GitHub Actions 自动发布

将 `src-tauri/Cargo.toml` 和 `src-tauri/tauri.conf.json` 更新为相同版本，再推送对应的 `v<版本>` 标签：

```powershell
git tag v0.1.1
git push origin v0.1.1
```

工作流会检查版本、运行测试、构建 Windows x64 便携包，并发布到 [Releases](https://github.com/BionicMagua/agy-switch/releases)。附件包括 ZIP 和 `SHA256SUMS.txt`；使用仓库自带的 `GITHUB_TOKEN`，无需额外配置密钥。

## 致谢

界面排版与视觉设计参考了 [Magpie](https://github.com/yetone/magpie)（MIT License）。
