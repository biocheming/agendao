#!/usr/bin/env python3

import json
import sys


def main() -> int:
    payload = json.load(sys.stdin)
    result = {
        "tool": "dock_pose",
        "status": "example",
        "received": payload,
    }
    print(json.dumps(result, ensure_ascii=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
