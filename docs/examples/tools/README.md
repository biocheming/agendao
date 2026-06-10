# Tool Config Examples

文档基线：v2026.6.10（更新日期：2026-06-10）

这个目录只演示一件事：如何用 `toolImports` + 外部 `tools.jsonc` 管理外部 tool catalog，并让示例路径、目录推断、`catalog-only` / `executable` 语义与当前 loader 行为保持一致。

不要把外部 tool 配置继续堆回主配置。`agendao.jsonc` 里只保留导入入口，真正的 tool 清单放到独立 `tools.jsonc` 文件。

## What Is Actually Covered

- `agendao.jsonc.example`
  - repo 根最小入口：主配置只保留一个 `toolImports`
- `single-file/`
  - 一个 `tools.jsonc` 同时承载 `source` / `catalog` / `execution`
- `split-imports/`
  - 主配置导入多个 catalog 文件，按导入顺序合并
- `directory-infer/`
  - `catalog` 留空，由 `tools/<domain>/<family>/<subfamily>/...` 目录结构回填缺失层级
- `partial-backfill/`
  - 只显式写一部分 `catalog`，其余层级从目录结构补齐
- `catalog-only/`
  - 只有发现/分类能力，没有执行声明；可被搜索和描述，不可直接执行

这些示例都对着当前仓库里的真实文件树：

- `single-file/tools.jsonc` 的 `source.path` / `execution.entry` / `arguments_schema_ref` 都能在同目录下找到目标文件
- `split-imports/` 真实演示了两个独立 catalog 文件的合并
- `directory-infer/tools/catalog.jsonc` 刻意把 `catalog` 留空，依赖 `tools/cadd/molecular_docking/protein_ligand/...` 回填
- `partial-backfill/tools/catalog.jsonc` 只锁 `family = scoring`，把 `domain` / `subfamily` 交给目录推断
- `catalog-only/tools/catalog.jsonc` 没有 `execution` 块，当前只能进入 `tool_catalog_search` / `tool_catalog_describe`

## Minimal entry

如果你的主配置在 repo 根，可以先从这个最小入口开始：

```jsonc
{
  "toolImports": [
    "./docs/examples/tools/single-file/tools.jsonc"
  ]
}
```

完整文件见：`./agendao.jsonc.example`。这个入口是 repo 根路径语义，不适合原样复制到别的目录。

## Which pattern to use

1. 工具数量少、先跑通一条链路：用 `single-file/`
2. 一个 domain 下有多类工具：用 `split-imports/`
3. 你已经把目录层级整理成 `tools/<domain>/<family>/<subfamily>/...`：用 `directory-infer/`
4. 你想显式锁一层分类，其余交给目录自动补：用 `partial-backfill/`
5. 你暂时只想把 catalog 暴露给模型，还不想开放执行：用 `catalog-only/`

## Ground truth

- 主配置字段名是 `toolImports`
- `toolImports` 里的相对路径，按“声明它的配置文件所在目录”解析
- `tools.jsonc` 内的 `source.path` / `source.manifest` / `execution.entry` / `execution.arguments_schema_ref`
  都按“当前 `tools.jsonc` 所在目录”解析
- 第一版 `execution.kind` 只支持 `script_runner`
- 声明了 `execution` 就必须有 `execution.entry`
- 没有 `execution` 的条目会被当成 `catalog-only`

这些规则的字段依据见：[configuration.md](../../configuration.md)

## Field meaning

- `source.path`
  - 描述这个 tool 的来源文件或主要实现位置
  - 会进入发现/描述面，便于模型和用户理解 provenance
- `catalog`
  - `domain` / `family` / `subfamily` / `tags` / `provenance`
  - 用于大 catalog 下的分层治理和渐进式暴露
- `execution`
  - 当前是否允许 `tool_catalog_call` 走外部执行适配器真正调用
- `arguments_schema_ref`
  - 可选参数 schema 文件；适合把大参数结构移出主清单

## Copying notes

- 这些示例文件放在 `docs/examples/tools/` 下，路径是为了演示“相对谁解析”
- 如果你把示例复制到 `.agendao/agendao.jsonc` 或 `~/.config/agendao/agendao.jsonc`，记得同步改相对路径
- 推荐最终目录形态：

```text
.agendao/
  agendao.jsonc
  tools/
    cadd/
      docking/
        tools.jsonc
      md/
        tools.jsonc
```

## Example map

- 最小单文件：`single-file/tools.jsonc`
- 多文件导入：`split-imports/agendao.jsonc.example`
- 全目录推断：`directory-infer/tools/catalog.jsonc`
- 部分回填：`partial-backfill/tools/catalog.jsonc`
- 只暴露 catalog：`catalog-only/tools/catalog.jsonc`

## What these examples do not claim

- 它们不保证外部 tool 的业务参数一定合理，只保证配置层字段、相对路径和 catalog 语义是当前合法形态
- 它们不替代 `tool_catalog_search -> tool_catalog_describe -> tool_catalog_call` 的发现/执行流程说明；运行时入口仍以 `configuration.md` 和产品文档为准
- `catalog-only` 示例不是“半配置错误”，而是刻意演示“大 catalog 先暴露发现面、后开放执行面”的治理方式
