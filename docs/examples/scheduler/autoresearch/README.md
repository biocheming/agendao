# Autoresearch Workflow Examples

这个目录放 scheduler 语义下的 autoresearch 工作流示例，而不是公开 preset。

- `book-authoring.example.jsonc`
  - 一个完整的长文写作 workflow 配置，展示 `workflow.kind = "autoresearch"` 在 scheduler profile 里的接法
- `verify.sh`
  - 对章节结构、长度、图表和标题约束做验证
- `guard.sh`
  - 对占位文本、代码 fence 和缺失 Summary 做 guard
- `skill/`
  - 对应的 machine-parsed autoresearch skill 示例

它和 `verifier/`、`pso/` 并列存在，是为了把“内置 preset 示例”和“workflow / topology 示例”分开。
