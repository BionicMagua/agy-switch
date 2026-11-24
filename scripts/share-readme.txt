AGY Switch —— Antigravity 多账号一键切换

在 Antigravity（Google 的桌面版 AI 编辑器）里存好几个账号，点一下就换，不用反复登出登入。
只改你自己电脑上的登录凭据，不联网、不上传任何东西。

【准备】
- Windows 10/11，已安装 Antigravity 桌面版。
- 新建一个只放它的文件夹（例如 D:\Tools\agy-switch），把 agy-switch.exe 放进去再双击。
  不要在压缩包里直接运行，也别直接放在“下载”文件夹里运行。
- 程序没有数字签名，第一次打开可能提示“Windows 已保护你的电脑”：点「更多信息」→「仍要运行」。
  如果提示的是“智能应用控制已阻止”，就没有“仍要运行”可点，这台电脑目前运行不了它。
- 正常双击即可，不要右键“以管理员身份运行”。
- 提示“低完整性级别”：将 ZIP 解压到新文件夹后再运行。
- 提示 WebView2“缓存仍被占用”：完全退出工具后重开；仍失败可重启 Windows。
- 确实未安装 Microsoft Edge WebView2 Runtime 时，再从微软官网安装。

【第一次用】
1. 在 Antigravity 里正常登录账号 A。
2. 打开 AGY Switch，点底部「保存当前」，起个名字（比如“A”），回车。
3. 点「登录新账号」：它会关掉 Antigravity，清掉本机登录凭据，再把 Antigravity 打开到登录页。
4. 在 Antigravity 里登录账号 B，回到 AGY Switch，点「保存当前」存成“B”。
5. 以后点某个账号右边的开关就能切过去，Antigravity 会自动关闭并重启。

【注意】
- 换号请用「登录新账号」，不要在 Antigravity 里点“退出登录”：那会在服务器上注销，已保存的档案可能随之失效。
- 切换会强制关闭 Antigravity（开着时会先问你），先保存好文件，确认没有正在跑的 Agent 任务。
- 当前登录还没保存成档案时，切换和「登录新账号」都会被拒绝，先点「保存当前」。
- 「设置」里的“保存时包含 ADC 文件”只有在 Antigravity 用 ADC（gcloud 应用默认凭据）登录时才需要打开；
  打开后每次切换都会改写 %APPDATA%\gcloud\application_default_credentials.json。
- Antigravity 开着 ADC 登录时，「登录新账号」和切到不含 ADC 的档案都会被拒绝，因为启动时 ADC 会自动登录回去。
- 想截图给别人看时，点右上方「隐藏账号」，邮箱、档案名和 GCP 项目会被模糊。

【出问题时】
- 一直提示“降级文件 中的登录凭据不是有效 JSON”：关掉 Antigravity，删除
  %USERPROFILE%\.gemini\jetski-standalone-oauth-token 这个文件，再重新打开工具。
- 提示“检测到以管理员身份运行的 Antigravity”：手动完全退出 Antigravity（包括托盘图标）再试。
- 打不开时，错误会写在 %LOCALAPPDATA%\agy-switch\startup.log。

【数据放在哪】
- 档案用 Windows 自带的 DPAPI 加密，存在 %LOCALAPPDATA%\agy-switch\profiles，只有你这个 Windows 账户能解开，拷到别的电脑上也用不了。
- 不想用了：删掉 agy-switch.exe 和 %LOCALAPPDATA%\agy-switch 文件夹即可，Antigravity 本身不受影响。
