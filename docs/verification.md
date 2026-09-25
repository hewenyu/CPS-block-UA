# 0.1.0 本地验证记录

验证日期：2026-09-25。CPS 最新稳定 Release 为 v3.14.1；SDK 与打包 CLI 均来自提交 `21c0711a6693372869475d1bec3037ef4b08525f`。

| 目标 | 构建/执行环境 | 结果 |
| --- | --- | --- |
| Linux AMD64 | ARM64 Rust 交叉编译；AMD64 Debian 12 容器模拟执行测试程序与插件子进程 | 16 项 Rust 测试通过，可安装包已生成 |
| Linux ARM64 | ARM64 Debian 12 容器原生编译执行 | 16 项 Rust 测试通过，可安装包已生成 |
| macOS ARM64 | 本机原生编译执行 | 16 项 Rust 测试通过，可安装包已生成 |

16 项测试包含 7 项策略测试与 9 项 SDK/进程会话测试。覆盖官方产品格式（含 `codex_exec`、`codex-tui`）、非匹配 UA、缺失/重复/非法 UA、模型查询豁免、自定义规则完整匹配、无效配置、协议注册、HTTP/SSE/WS 原正文 Preserve 与 opaque 响应句柄透传、下游故障、取消、日志失败/超时，以及 32 并发日志挂起不阻塞 observe 放行。

发布版本脚本另有 2 项 Python 测试，通过稳定/预发布版本同步、不改变锁定依赖、拒绝非法标签且不写文件等场景。执行环境 Python 3.11；CI 固定 Python 3.12。

其他已通过检查：

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --locked -- -D warnings`
- GitHub Actions `actionlint`
- `bash -n scripts/package.sh` 与 `git diff --check`
- 三个平台归档的整包 SHA-256、包内二进制 SHA-256、可执行权限、真实二进制架构，以及配置 schema 与作者清单一致性。

本机 Docker 的 AMD64 模拟器运行 Rust 编译器出现 SIGSEGV，使用仓库提供的 `--docker-cross` 流程完成验证；不是跳过 AMD64 测试。两个 Linux 二进制最高 GLIBC 符号需求均为 2.34，实际运行测试基线为 2.36。

尚未执行：GitHub 远端工作流/Release 发布、真实 CPS 安装与绑定、经 New API 的真实模型请求与后续轮次。没有修改生产 CPS、New API 或 Nginx。
会话测试证明插件的 SDK 交互与放行/拒绝决策；CPS 对外 HTTP/WS 错误编码结论来自 v3.14.1 源码，尚未作为已安装插件进行端到端验收。

`dist/` 为本地生成文件，已加入忽略规则。推送源码与 `v*` 标签后，Actions 将按标签版本重新构建附件并发布，Release 附件摘要应以那次构建为准。
