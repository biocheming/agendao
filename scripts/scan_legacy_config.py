#!/usr/bin/env python3
"""
Scan legacy OpenCode / legacy CLI tool directories and config files, then print a
agendao.jsonc-compatible fragment for initial migration.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    tomllib = None  # type: ignore[assignment]


PLUGIN_EXTENSIONS = {".js", ".mjs", ".cjs", ".ts"}


@dataclass
class ConfigHit:
    path: Path
    keys: List[str]


def strip_json_comments(text: str) -> str:
    out: List[str] = []
    i = 0
    in_string = False
    escape = False
    in_line_comment = False
    in_block_comment = False

    while i < len(text):
        ch = text[i]
        nxt = text[i + 1] if i + 1 < len(text) else ""

        if in_line_comment:
            if ch == "\n":
                in_line_comment = False
                out.append(ch)
            i += 1
            continue

        if in_block_comment:
            if ch == "*" and nxt == "/":
                in_block_comment = False
                i += 2
                continue
            if ch in ("\n", "\r"):
                out.append(ch)
            i += 1
            continue

        if in_string:
            out.append(ch)
            if escape:
                escape = False
            elif ch == "\\":
                escape = True
            elif ch == '"':
                in_string = False
            i += 1
            continue

        if ch == '"':
            in_string = True
            out.append(ch)
            i += 1
            continue

        if ch == "/" and nxt == "/":
            in_line_comment = True
            i += 2
            continue

        if ch == "/" and nxt == "*":
            in_block_comment = True
            i += 2
            continue

        out.append(ch)
        i += 1

    return "".join(out)


def normalize_path(path: Path) -> Path:
    return path.expanduser().resolve(strict=False)


def to_user_path(path: Path, home: Path) -> str:
    normalized = normalize_path(path)
    home = normalize_path(home)
    try:
        rel = normalized.relative_to(home)
        return f"~/{rel.as_posix()}"
    except ValueError:
        return normalized.as_posix()


def walk_files(root: Path, max_depth: int) -> Iterable[Path]:
    root = normalize_path(root)
    if not root.is_dir():
        return
    root_depth = len(root.parts)

    for current, dirs, files in os.walk(root):
        current_path = Path(current)
        depth = len(current_path.parts) - root_depth
        if depth >= max_depth:
            dirs[:] = []
        for name in files:
            yield current_path / name


def has_plugin_files(root: Path) -> bool:
    for file in walk_files(root, max_depth=4):
        if file.suffix.lower() in PLUGIN_EXTENSIONS:
            return True
    return False


def has_skill_files(root: Path) -> bool:
    for file in walk_files(root, max_depth=6):
        if file.name == "SKILL.md":
            return True
    return False


def load_jsonc(path: Path) -> Optional[Dict[str, Any]]:
    try:
        content = path.read_text(encoding="utf-8")
    except Exception:
        return None
    try:
        data = json.loads(strip_json_comments(content))
    except Exception:
        return None
    return data if isinstance(data, dict) else None


def load_toml(path: Path) -> Optional[Dict[str, Any]]:
    if tomllib is None:
        return None
    try:
        content = path.read_text(encoding="utf-8")
    except Exception:
        return None
    try:
        data = tomllib.loads(content)
    except Exception:
        return None
    return data if isinstance(data, dict) else None


def load_config_object(path: Path) -> Optional[Dict[str, Any]]:
    ext = path.suffix.lower()
    name = path.name.lower()

    loaders = []
    if ext in {".toml"} or name == "config":
        loaders.extend([load_toml, load_jsonc])
    else:
        loaders.extend([load_jsonc, load_toml])

    for loader in loaders:
        data = loader(path)
        if data is not None:
            return data
    return None


def collect_declared_paths(
    payload: Dict[str, Any],
    config_path: Path,
) -> Tuple[List[Path], List[Path]]:
    plugin_candidates: List[Path] = []
    skill_candidates: List[Path] = []
    base = config_path.parent

    def add_if_str(items: Sequence[Any], target: List[Path]) -> None:
        for item in items:
            if isinstance(item, str) and item.strip():
                target.append(resolve_declared_path(base, item.strip()))

    for key in ("plugin_paths", "pluginPaths"):
        obj = payload.get(key)
        if isinstance(obj, dict):
            add_if_str(list(obj.values()), plugin_candidates)

    for key in ("skill_paths", "skillPaths"):
        obj = payload.get(key)
        if isinstance(obj, dict):
            add_if_str(list(obj.values()), skill_candidates)

    skills = payload.get("skills")
    if isinstance(skills, dict):
        paths = skills.get("paths")
        if isinstance(paths, list):
            add_if_str(paths, skill_candidates)

    return plugin_candidates, skill_candidates


def resolve_declared_path(base: Path, raw: str) -> Path:
    path = Path(raw).expanduser()
    if path.is_absolute():
        return normalize_path(path)
    return normalize_path(base / path)


def uniq_paths(paths: Iterable[Path]) -> List[Path]:
    out: List[Path] = []
    seen = set()
    for raw in paths:
        path = normalize_path(raw)
        key = path.as_posix()
        if key in seen:
            continue
        seen.add(key)
        out.append(path)
    return out


def legacy_roots(cwd: Path, home: Path) -> List[Tuple[str, Path]]:
    return [
        ("cwd_opencode", cwd / ".opencode"),
        ("home_opencode", home / ".opencode"),
        ("config_opencode", home / ".config" / "opencode"),
        ("data_opencode", home / ".local" / "share" / "opencode"),
        ("cache_opencode", home / ".cache" / "opencode"),
    ]


def candidate_config_files(cwd: Path, home: Path) -> List[Path]:
    roots = [
        cwd,
        cwd / ".opencode",
        home / ".opencode",
        home / ".config" / "opencode",
    ]
    names = [
        "opencode.jsonc",
        "opencode.json",
        "config.jsonc",
        "config.json",
        "settings.jsonc",
        "settings.json",
        "config",
    ]

    candidates: List[Path] = []
    candidates.append(home / ".opencode.json")
    for root in roots:
        for name in names:
            candidates.append(root / name)
    return uniq_paths(candidates)


def extract_model(payload: Dict[str, Any]) -> Optional[str]:
    provider = payload.get("provider")
    model = payload.get("model")
    if isinstance(provider, str) and isinstance(model, str):
        if "/" in model:
            return model
        return f"{provider}/{model}"

    for key in ("model", "default_model", "defaultModel"):
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return None


def extract_small_model(payload: Dict[str, Any]) -> Optional[str]:
    for key in ("small_model", "smallModel"):
        value = payload.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return None


def extract_mcp(payload: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    for key in ("mcp", "mcpServers"):
        value = payload.get(key)
        if isinstance(value, dict):
            return value
    return None


def extract_providers(payload: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    """Extract the provider configuration map from a legacy config."""
    for key in ("provider", "providers"):
        value = payload.get(key)
        if isinstance(value, dict) and value:
            return value
    return None


def extract_plugin_specs(payload: Dict[str, Any]) -> List[str]:
    """Extract the plugin npm package specs (e.g. 'oh-my-opencode@latest').

    Also supports the new map format: {"name": {"type": "npm", ...}}.
    """
    value = payload.get("plugin")
    if isinstance(value, list):
        return [s for s in value if isinstance(s, str) and s.strip()]
    return []


def specs_to_plugin_map(specs: List[str]) -> Dict[str, Any]:
    """Convert legacy string specs to the new plugin map format."""
    plugin_map: Dict[str, Any] = {}
    for spec in specs:
        if spec.startswith("file://"):
            path = spec[len("file://"):]
            name = Path(path).stem or "plugin"
            plugin_map[name] = {"type": "file", "path": path}
        else:
            # npm spec: "pkg@version" or "@scope/pkg@version"
            name, version = _parse_npm_spec(spec)
            key = name.lstrip("@").replace("/", "-")
            entry: Dict[str, Any] = {"type": "npm", "package": name}
            if version and version != "*":
                entry["version"] = version
            plugin_map[key] = entry
    return plugin_map


def _parse_npm_spec(spec: str) -> Tuple[str, Optional[str]]:
    """Parse 'pkg@version' into (name, version)."""
    if spec.startswith("@"):
        at_idx = spec.find("@", 1)
        if at_idx > 0:
            return spec[:at_idx], spec[at_idx + 1:]
        return spec, None
    at_idx = spec.find("@")
    if at_idx > 0:
        return spec[:at_idx], spec[at_idx + 1:]
    return spec, None


def suggest_key(prefix: str, path: Path, used: set[str]) -> str:
    parts_lower = [part.lower() for part in path.parts]
    source = "external"
    if "opencode" in parts_lower:
        source = "opencode"
    elif "claude" in parts_lower:
        source = "claude"

    tail = path.name.lower()
    if tail in {"plugins", "plugin", "skills"} and path.parent.name:
        tail = path.parent.name.lower()
    tail = "".join(ch if ch.isalnum() else "-" for ch in tail).strip("-") or "path"

    base = f"{source}-{tail}-{prefix}"
    key = base
    idx = 2
    while key in used:
        key = f"{base}-{idx}"
        idx += 1
    used.add(key)
    return key


def parse_legacy_configs(config_files: List[Path]) -> Tuple[Optional[str], Optional[str], Dict[str, Any], Dict[str, Any], List[ConfigHit], List[Path], List[Path], List[str]]:
    model: Optional[str] = None
    small_model: Optional[str] = None
    mcp: Dict[str, Any] = {}
    providers: Dict[str, Any] = {}
    hits: List[ConfigHit] = []
    declared_plugins: List[Path] = []
    declared_skills: List[Path] = []
    plugin_specs: List[str] = []

    for file in config_files:
        if not file.is_file():
            continue
        payload = load_config_object(file)
        if payload is None:
            continue

        keys: List[str] = []
        found_model = extract_model(payload)
        found_small_model = extract_small_model(payload)
        found_mcp = extract_mcp(payload)
        found_providers = extract_providers(payload)
        found_plugin_specs = extract_plugin_specs(payload)

        if model is None and found_model:
            model = found_model
            keys.append("model")
        if small_model is None and found_small_model:
            small_model = found_small_model
            keys.append("small_model")
        if found_mcp:
            for name, cfg in found_mcp.items():
                if name not in mcp:
                    mcp[name] = cfg
            if found_mcp:
                keys.append("mcp")
        if found_providers:
            for pid, pcfg in found_providers.items():
                if pid not in providers:
                    providers[pid] = pcfg
            keys.append("provider")
        if found_plugin_specs:
            seen = set(plugin_specs)
            for spec in found_plugin_specs:
                if spec not in seen:
                    plugin_specs.append(spec)
                    seen.add(spec)
            keys.append("plugin")

        plugin_paths, skill_paths = collect_declared_paths(payload, file)
        declared_plugins.extend(plugin_paths)
        declared_skills.extend(skill_paths)
        if plugin_paths:
            keys.append("plugin_paths")
        if skill_paths:
            keys.append("skill_paths")

        if keys:
            hits.append(ConfigHit(path=file, keys=sorted(set(keys))))

    return model, small_model, mcp, providers, hits, declared_plugins, declared_skills, plugin_specs


def is_cache_path(path: Path) -> bool:
    """Return True if path is under a cache directory (auto-managed, not user plugin sources)."""
    parts_lower = [p.lower() for p in path.parts]
    return ".cache" in parts_lower


def model_covered_by_providers(model_str: str, providers: Dict[str, Any]) -> bool:
    """Check if a 'provider/model' string is already covered by the providers dict."""
    if "/" in model_str:
        provider_id = model_str.split("/", 1)[0]
        return provider_id in providers
    return False


def discover_plugin_roots(roots: Sequence[Tuple[str, Path]], extra_candidates: Sequence[Path]) -> List[Path]:
    candidates: List[Path] = []
    for _, root in roots:
        candidates.extend([root / "plugins", root / "plugin"])
    candidates.extend(extra_candidates)

    valid = []
    for path in uniq_paths(candidates):
        if is_cache_path(path):
            continue
        if path.is_dir() and has_plugin_files(path):
            valid.append(path)
    return valid


def discover_skill_roots(roots: Sequence[Tuple[str, Path]], extra_candidates: Sequence[Path]) -> List[Path]:
    candidates: List[Path] = []
    for _, root in roots:
        candidates.append(root / "skills")
    candidates.extend(extra_candidates)

    valid = []
    for path in uniq_paths(candidates):
        if path.is_dir() and has_skill_files(path):
            valid.append(path)
    return valid


def build_fragment(
    plugin_paths: Sequence[Path],
    skill_paths: Sequence[Path],
    model: Optional[str],
    small_model: Optional[str],
    mcp: Dict[str, Any],
    providers: Dict[str, Any],
    plugin_specs: Sequence[str],
    home: Path,
) -> Dict[str, Any]:
    fragment: Dict[str, Any] = {}

    if plugin_paths:
        used = set()
        plugin_map: Dict[str, str] = {}
        for path in plugin_paths:
            key = suggest_key("plugins", path, used)
            plugin_map[key] = to_user_path(path, home)
        fragment["plugin_paths"] = plugin_map

    if skill_paths:
        used = set()
        skill_map: Dict[str, str] = {}
        for path in skill_paths:
            key = suggest_key("skills", path, used)
            skill_map[key] = to_user_path(path, home)
        fragment["skill_paths"] = skill_map

    if model and not model_covered_by_providers(model, providers):
        fragment["model"] = model
    if small_model and not model_covered_by_providers(small_model, providers):
        fragment["small_model"] = small_model
    if providers:
        fragment["provider"] = providers
    if plugin_specs:
        fragment["plugin"] = specs_to_plugin_map(plugin_specs)
    if mcp:
        fragment["mcp"] = mcp

    return fragment


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Scan OpenCode and other legacy CLI tool directories and config files, then emit a "
            "agendao.jsonc fragment for plugin_paths/skill_paths/model/mcp."
        )
    )
    parser.add_argument(
        "--cwd",
        type=Path,
        default=Path.cwd(),
        help="Project directory used for local legacy config scanning (default: current directory). "
             "Legacy OpenCode-style directories under this path are scanned for migration.",
    )
    parser.add_argument(
        "--home",
        type=Path,
        default=Path.home(),
        help="Home directory used for ~/.opencode and other legacy CLI tool config scanning (default: current user's home).",
    )
    parser.add_argument(
        "--json-only",
        action="store_true",
        help="Print only the JSON fragment (no summary text).",
    )
    parser.add_argument(
        "--write",
        type=Path,
        help="Write the generated JSON fragment to this file.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    cwd = normalize_path(args.cwd)
    home = normalize_path(args.home)

    roots = legacy_roots(cwd, home)
    configs = candidate_config_files(cwd, home)
    model, small_model, mcp, providers, hits, declared_plugins, declared_skills, plugin_specs = parse_legacy_configs(configs)

    plugin_roots = discover_plugin_roots(roots, declared_plugins)
    skill_roots = discover_skill_roots(roots, declared_skills)
    fragment = build_fragment(plugin_roots, skill_roots, model, small_model, mcp, providers, plugin_specs, home)
    fragment_text = json.dumps(fragment, ensure_ascii=False, indent=2)

    if args.write:
        out = normalize_path(args.write)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(fragment_text + "\n", encoding="utf-8")

    if args.json_only:
        print(fragment_text)
        return 0

    print("Legacy scan complete.")
    print(f"- CWD scanned: {to_user_path(cwd, home)}")
    print(f"- Home scanned: {to_user_path(home, home)}")
    print(f"- Plugin roots found: {len(plugin_roots)}")
    for path in plugin_roots:
        print(f"  - {to_user_path(path, home)}")
    print(f"- Skill roots found: {len(skill_roots)}")
    for path in skill_roots:
        print(f"  - {to_user_path(path, home)}")
    print(f"- Plugin packages: {len(plugin_specs)}")
    for spec in plugin_specs:
        print(f"  - {spec}")
    print(f"- Providers found: {len(providers)}")
    for pid, pcfg in providers.items():
        model_count = len(pcfg.get("models", {})) if isinstance(pcfg, dict) else 0
        print(f"  - {pid} ({model_count} model(s))")
    print(f"- Legacy config hits: {len(hits)}")
    for hit in hits:
        print(f"  - {to_user_path(hit.path, home)} ({', '.join(hit.keys)})")

    print("\nSuggested agendao.jsonc fragment:\n")
    print(fragment_text)

    if not fragment:
        print(
            "\nNo legacy entries were detected. You can still keep agendao defaults and add paths manually.",
            file=sys.stderr,
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
