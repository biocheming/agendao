# 配置 examples

这些是当前 schema 对齐的最小例子。它们只展示一种做法，不会把所有可选字段塞进一个“万能配置”。

## 文件

- `agendao.minimal.jsonc.example`：最小 provider + model 配置
- `agendao.permission.jsonc.example`：全局 permission 规则与 agent 级规则
- `agendao.ollama.jsonc.example`：本地 Ollama（必须先启动 Ollama）
- `agendao.context-docs.jsonc.example`：只配置 context docs registry 路径

## 使用

复制到项目根目录时，文件名改为 `agendao.jsonc`，并根据自己的 endpoint、model ID 和环境变量修改。相对路径以配置文件所在目录为基准。

```bash
cp docs/examples/configuration/agendao.minimal.jsonc.example agendao.jsonc
agendao config
```

`agendao config` 只验证配置能否解析；真正调用模型还需要有效认证、可达 endpoint 和正确模型 ID。

## 认证原则

不要把真实 API key 放进 example。推荐在 shell 中设置 provider 的环境变量，或使用当前 AgenDao 的认证存储。配置字段名以 `docs/configuration.md` 和 `docs/agendao_config.schema.json` 为准。
