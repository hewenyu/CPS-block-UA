# CPS-block-UA

为 [Codex Proxy RS](https://github.com/zyycn/codex-proxy-rs) 提供入站 User-Agent 访问控制。
匹配请求原样交给下游，不转换 Chat → Responses，不改写模型、请求头或请求正文，不读取或重新编码响应流。

当前插件版本 **0.1.0**，适配 **CPS v3.14.1**（2026-09-25 核对的最新稳定版）。
只使用公开 Rust SDK，固定源码提交 `21c0711a6693372869475d1bec3037ef4b08525f`；宿主升级后需要核对合同并更新兼容范围，不声明未经验证的所有未来版本。

## 行为

- `observe`：默认模式。不匹配时尝试记录策略事件，仍然放行。
- `enforce`：不匹配时返回 SDK `Rejected`，不执行 `next`，不调用模型上游。
- 默认允许 `codex_cli_rs/版本`、`codex-cli/版本`、`codex-tui/版本`、`codex_exec/版本`、`codex_vscode/版本`、`Codex Desktop/版本` 开头的 UA，接受平台/终端后缀与预发行版本。产品名称大小写敏感。
- 缺失、空、重复、超过 4096 字节、非 ASCII 或带控制字符的 UA 被视为不匹配。首尾 HTTP 空白只在匹配时忽略，转发值不变。
- 默认豁免 `/v1/models` 和 `/v1/models/{id}` 的 UA 检查；CPS 的 Key 认证仍生效。

UA 由客户端自行声明，可以伪造；本插件用于筛选客户端类型，不能证明“官方客户端身份”。默认规则是可配置的格式白名单，新版客户端更改格式时需先观察再调整。

| 传输 | 拒绝行为 |
| --- | --- |
| HTTP JSON / SSE | CPS 返回 HTTP 403，`invalid_request_error` / `policy_denied` |
| Responses WebSocket | 握手成功后，在每个 `response.create` 进入请求中间件时检查握手 UA；返回带 403 状态的错误事件，不执行该次模型请求，连接保持 |

插件不控制 WebSocket 握手或主动关闭连接。错误由 CPS 统一编码，对外不保留自定义 `codex_client_required` 错误码。允许请求沿用 SDK 的原正文保留与不透明响应句柄，JSON、SSE、WS 交付由宿主负责。

## 安装与绑定

1. 在 CPS 插件管理中上传对应服务器/容器架构的 `dist/*.tar.gz`，核对 `.sha256`。Linux AMD64 与 ARM64 是不同安装包。
2. 核对插件 ID `hewenyu.cps-block-ua`、能力 `middleware` / `request`、权限 `requests`。插件是与宿主同权限运行的 `trustedProcess`。
3. 默认配置会准备为 `observe`。**默认绑定的空范围表示全范围**；将绑定的客户端 Key 限定为 New API 渠道使用的 CPS Key，其他范围按实际需要选择。若有其他改写请求头的插件，让本插件先执行。
4. 使用真实 Codex 客户端，经 New API 请求 CPS，检查 CPS 请求记录中的入站 UA。New API 必须保留原始 `User-Agent`，不能统一替换成固定 Codex UA，否则本插件失去区分客户端的依据。
5. 确认规则后应用 `config/enforce.json`，绑定故障策略选择 `reject`。配置无效时插件启动失败；故障策略影响运行故障是否放行，显式策略拒绝始终拒绝。

这些步骤不需要修改 Nginx。本仓库的构建脚本不执行安装、启用、推送或发布。

**New API 后台测试通常发送 `Go-http-client/1.1`，强制模式会拒绝它。** 模型列表豁免不等于生成测试豁免；应以真实 Codex 请求验证可用性，不要为了后台测试放行 Go 客户端。

## 配置

```json
{
  "mode": "enforce",
  "allow_models": true
}
```

省略 `allow_patterns` 使用内置规则。显式设置则替换全部默认规则，例如只允许 CLI：

```json
{
  "mode": "enforce",
  "allow_models": false,
  "allow_patterns": [
    "codex_cli_rs/[0-9]+(?:\\.[0-9]+){1,3}(?:[-+][0-9A-Za-z.-]+)?(?: [ -~]*)?"
  ]
}
```

使用 Rust `regex` 语法，插件自动对每条表达式做完整匹配，不是子串搜索；`(?i)` 可显式启用大小写不敏感。
最多 32 条规则，每条最多 1024 字节，启动时一次编译并限制编译内存。无效表达式、空列表、未知字段或错误类型直接报错。
不要配置 `.*` 等宽泛规则，除非确实希望放行所有格式合法的 UA。

日志仅通过 `host.log` 提交固定策略事件及 mode/decision/reason，不提交原始 UA、Key、请求正文或认证头。
日志失败、被限流或等待超过 100ms 不改变判定。CPS 会脱敏事件和字段，日志不保证保存明文原因或每次事件；核对真实 UA 请使用 CPS 自身请求记录。
stdout 专用于 SDK 协议。

## 开发与打包

需要 Rust **1.97.0**（`rust-toolchain.toml`）、Git；发布版本脚本与对应测试需要 Python **3.11+**。跨平台本地构建还需运行中的 Docker（Docker Desktop 支持 AMD64 模拟执行）。

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
python3 -m unittest discover -s tests -p 'test_*.py'

# Linux 包：在固定的 Debian 12 / Rust 1.97 镜像内运行测试和编译。
bash scripts/package.sh x86_64-unknown-linux-gnu --docker
bash scripts/package.sh aarch64-unknown-linux-gnu --docker

# Apple Silicon 开发机原生包。
bash scripts/package.sh aarch64-apple-darwin
```

Apple Silicon 上若 Docker 的 AMD64 模拟器运行 `rustc` 崩溃，可用以下备用流程（另需 Python 3）。它在 ARM64 容器安装交叉编译器，随后在 AMD64 容器执行生成的测试程序，包括插件子进程 IPC 测试；不会只编译不验证。

```bash
bash scripts/package.sh x86_64-unknown-linux-gnu --docker-cross
```

Linux GNU 包以 glibc **2.36**（Debian 12）为构建与运行测试基线，不适用于 Alpine/musl。
本次两个 Linux 二进制的最高 GLIBC 符号要求为 **2.34**；更旧运行环境未验收。服务器 CPU 架构和 CPS 运行容器内的 libc 均需匹配，旧环境应先验证，必要时使用适当基线重新构建。
脚本把同一 SDK 提交的官方 `cpr-plugin` 安装到仓库 `.tools/`，产物输出到 `dist/`。
已有相同提交构建的 CLI 时，可设置 `CPR_PLUGIN=/absolute/path/to/cpr-plugin`。
原生构建可指定其他已配置编译器的 target；脚本会在该 target 上执行测试，不能只修改包的平台标签。

## GitHub 标签自动发布

推送 `v*` 标签后，GitHub Actions 自动完成：校验标签 → 同步构建版本 → 格式/静态检查和测试 → Linux AMD64、Linux ARM64、macOS ARM64 打包 → 校验全部 SHA-256 → 创建带安装包附件的 GitHub Release。
标签是发布版本的来源；工作流仅在临时 checkout 中同步 `Cargo.toml`、根包的 `Cargo.lock` 和 `plugin.json`，不回写分支、不升级依赖。

```bash
# 先将源码和工作流提交、推送到仓库，再为该提交打标签。
git tag v0.1.0
git push origin v0.1.0
```

接受 `v1.2.3` 和 `v1.2.3-rc.1` 等 SemVer 标签。带预发行后缀的 Release 自动标记为预发布；CPS 安装预发布包时需明确指定标签并允许预发布。稳定标签生成正式 Release，可从 CPS 的 GitHub 安装入口选择。
任一检查或平台失败时不发布；已存在的 Release 不自动覆盖，修改发布内容请用新标签。
普通分支/PR 只执行检查；手动 `workflow_dispatch` 可打包供下载，选择标签运行时也会触发对应 Release。
Actions 发布任务使用仓库自带的 `GITHUB_TOKEN`，仅该任务授予 `contents: write`，无需另配个人 Token。

测试覆盖策略边界、配置错误、真实 SDK IPC 注册/调用、原正文与响应句柄保留、观察/拒绝及故障行为。
SDK 会话测试不等同于生产宿主安装验收。启用前还应通过真实 Codex 验证 HTTP/SSE、WS、compact 与后续轮次；在 CPS 未报告的入口或未绑定范围内，本插件不会生效。

本次本地交付的实际检查与限制见 [验证记录](docs/verification.md)。
