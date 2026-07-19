import { describe, expect, it } from "vitest";
import {
  filterSlashCommands,
  parseSlashCommandSubmission,
  slashCommandQueryFromText,
  type CommandApiSpec,
} from "./command";

function command(
  name: string,
  overrides: Partial<CommandApiSpec> = {},
): CommandApiSpec {
  return {
    name,
    description: `${name} description`,
    source: "Builtin",
    ...overrides,
  };
}

const COMMANDS: CommandApiSpec[] = [
  command("review", { aliases: ["rv"], description: "Review code changes" }),
  command("release", { description: "Cut a release" }),
  command("init", { description: "Create AGENTS.md" }),
];

describe("slashCommandQueryFromText", () => {
  it("returns the in-progress token for slash-prefixed single tokens", () => {
    expect(slashCommandQueryFromText("/")).toBe("");
    expect(slashCommandQueryFromText("/re")).toBe("re");
    expect(slashCommandQueryFromText("/review")).toBe("review");
  });

  it("returns null once the command token is complete or text is not a slash token", () => {
    expect(slashCommandQueryFromText("/review src/")).toBeNull();
    expect(slashCommandQueryFromText("hello /review")).toBeNull();
    expect(slashCommandQueryFromText(" /review")).toBeNull();
    expect(slashCommandQueryFromText("")).toBeNull();
    expect(slashCommandQueryFromText("plain text")).toBeNull();
  });
});

describe("filterSlashCommands", () => {
  it("returns all commands for an empty query", () => {
    expect(filterSlashCommands(COMMANDS, "")).toEqual(COMMANDS);
  });

  it("matches by name prefix, case-insensitively", () => {
    expect(filterSlashCommands(COMMANDS, "Re").map((item) => item.name)).toEqual([
      "release",
      "review",
    ]);
  });

  it("matches aliases", () => {
    expect(filterSlashCommands(COMMANDS, "rv").map((item) => item.name)).toEqual(["review"]);
  });

  it("ranks exact name matches before prefix matches", () => {
    expect(filterSlashCommands(COMMANDS, "review").map((item) => item.name)).toEqual([
      "review",
    ]);
    expect(filterSlashCommands(COMMANDS, "re").map((item) => item.name)).toEqual([
      "release",
      "review",
    ]);
  });

  it("returns an empty list when nothing matches", () => {
    expect(filterSlashCommands(COMMANDS, "zzz")).toEqual([]);
  });
});

describe("parseSlashCommandSubmission", () => {
  it("parses a command with arguments", () => {
    const submission = parseSlashCommandSubmission("/review src/lib", COMMANDS);
    expect(submission).not.toBeNull();
    expect(submission?.command).toBe("review");
    expect(submission?.args).toBe("src/lib");
    expect(submission?.text).toBe("/review src/lib");
    expect(submission?.spec.name).toBe("review");
  });

  it("resolves aliases to the canonical command name", () => {
    const submission = parseSlashCommandSubmission("/rv --fast", COMMANDS);
    expect(submission?.command).toBe("review");
    expect(submission?.args).toBe("--fast");
    expect(submission?.text).toBe("/review --fast");
  });

  it("parses a command without arguments", () => {
    const submission = parseSlashCommandSubmission("/init", COMMANDS);
    expect(submission?.command).toBe("init");
    expect(submission?.args).toBeUndefined();
    expect(submission?.text).toBe("/init");
  });

  it("trims surrounding whitespace and collapses trailing argument space", () => {
    const submission = parseSlashCommandSubmission("  /review   a  b  ", COMMANDS);
    expect(submission?.command).toBe("review");
    expect(submission?.args).toBe("a  b");
    expect(submission?.text).toBe("/review a  b");
  });

  it("returns null for unknown commands so the caller falls back to prompt", () => {
    expect(parseSlashCommandSubmission("/unknown arg", COMMANDS)).toBeNull();
  });

  it("returns null for non-slash text and for an empty command list", () => {
    expect(parseSlashCommandSubmission("just a prompt", COMMANDS)).toBeNull();
    expect(parseSlashCommandSubmission("/review", [])).toBeNull();
  });
});
