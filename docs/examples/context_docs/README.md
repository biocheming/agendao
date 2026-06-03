# Context Docs Examples

Files in this directory are the formal Phase 1 examples for the `context_docs` configuration path and registry layout.

## Files

- `agendao.json.example`: minimal JSON config example
- `agendao.jsonc.example`: minimal JSONC config example
- `context-docs-registry.schema.json`: registry schema
- `context-docs-registry.example.json`: registry example
- `context-docs-index.schema.json`: docs index schema
- `react-router.docs-index.example.json`: example docs index referenced by the registry
- `tokio.docs-index.example.json`: secondary example docs index referenced by the registry

## Minimal config

Point AgenDao config at an external registry file:

```jsonc
{
  "docs": {
    "contextDocsRegistryPath": "./docs/examples/context_docs/context-docs-registry.example.json"
  }
}
```

## Schema IDs

- Registry schema: `https://agendao.dev/schemas/context-docs-registry.schema.json`
- Docs index schema: `https://agendao.dev/schemas/context-docs-index.schema.json`

## Validation

Use the built-in read-only validator to verify the registry or a single docs index:

```bash
agendao debug docs validate
agendao debug docs validate --registry ./docs/examples/context_docs/context-docs-registry.example.json
agendao debug docs validate --index ./docs/examples/context_docs/react-router.docs-index.example.json
```

## Notes

- Keep `agendao.json` or `agendao.jsonc` minimal; only store the registry path there.
- Keep the actual registry in a separate file.
- `indexPath` may be relative to the registry file or absolute.
