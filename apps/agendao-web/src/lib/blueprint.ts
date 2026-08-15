export interface GeneratedAgentSpecRecord {
  id: string;
  base_agent: string;
  system_policy: string;
}

export interface SchedulerBlueprintRecord {
  schema: "v1";
  name: string;
  entry: string;
  nodes: Record<string, Record<string, unknown>>;
  limits: Record<string, number>;
  output: Record<string, unknown>;
}

export interface SessionBlueprintViewRecord {
  blueprint: SchedulerBlueprintRecord;
  generatedAgents: GeneratedAgentSpecRecord[];
  fingerprint: string;
  selectionSource: "user" | "heuristic" | "planner";
}

export const DIRECT_BLUEPRINT_STARTER: SchedulerBlueprintRecord = {
  schema: "v1",
  name: "web-blueprint",
  entry: "execute",
  nodes: {
    execute: {
      kind: "agent",
      agent: "build",
      skills: [],
      tools: [],
      required_model_capabilities: [],
      max_steps: 12,
      next: "done",
    },
    done: { kind: "end", result: "last-node" },
  },
  limits: {
    max_model_calls: 16,
    max_tool_calls: 48,
    max_total_tokens: 131072,
    max_wall_time_ms: 900000,
    max_parallelism: 2,
    max_graph_nodes: 16,
    max_graph_depth: 8,
    max_loop_iterations: 4,
    max_agent_steps: 12,
  },
  output: {
    format: "markdown",
    include_usage: true,
    include_artifact_refs: true,
  },
};

function isObject(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

export function parseBlueprintDocument(document: string): SchedulerBlueprintRecord {
  const value: unknown = JSON.parse(document);
  if (
    !isObject(value) ||
    value.schema !== "v1" ||
    typeof value.name !== "string" ||
    typeof value.entry !== "string" ||
    !isObject(value.nodes) ||
    !isObject(value.limits) ||
    !isObject(value.output)
  ) {
    throw new Error("Invalid SchedulerBlueprint v1 document");
  }
  return value as unknown as SchedulerBlueprintRecord;
}

export function formatBlueprintDocument(blueprint: SchedulerBlueprintRecord): string {
  return `${JSON.stringify(blueprint, null, 2)}\n`;
}
