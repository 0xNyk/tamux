import { describe, expect, it } from "vitest";
import { parseLeadingAgentDirective } from "./agentDirective";

describe("parseLeadingAgentDirective", () => {
  const known = ["weles", "veles", "rarog", "swarozyc", "perun", "mokosh", "dazhbog"];

  it("parses internal delegation", () => {
    expect(parseLeadingAgentDirective("!weles check claim", known)).toEqual({
      kind: "internal_delegate",
      agentAlias: "weles",
      body: "check claim",
    });
    expect(parseLeadingAgentDirective("!veles check claim", known)).toEqual({
      kind: "internal_delegate",
      agentAlias: "veles",
      body: "check claim",
    });
  });

  it("parses participant deactivation phrases", () => {
    expect(parseLeadingAgentDirective("@weles stop", known)).toEqual({
      kind: "participant_deactivate",
      agentAlias: "weles",
    });
    expect(parseLeadingAgentDirective("@weles return", known)).toEqual({
      kind: "participant_deactivate",
      agentAlias: "weles",
    });
  });

  it("returns null for unknown leading aliases", () => {
    expect(parseLeadingAgentDirective("@unknown inspect @src/file.ts", known)).toBeNull();
  });

  it("preserves file refs in the body", () => {
    expect(parseLeadingAgentDirective("@weles inspect @src/file.ts", known)).toEqual({
      kind: "participant_upsert",
      agentAlias: "weles",
      body: "inspect @src/file.ts",
    });
  });

  it("parses builtin persona aliases beyond weles and rarog", () => {
    expect(parseLeadingAgentDirective("@swarozyc review svarog output", known)).toEqual({
      kind: "participant_upsert",
      agentAlias: "swarozyc",
      body: "review svarog output",
    });
    expect(parseLeadingAgentDirective("@perun review the risky execution path", known)).toEqual({
      kind: "participant_upsert",
      agentAlias: "perun",
      body: "review the risky execution path",
    });
    expect(parseLeadingAgentDirective("@mokosh stabilize the workspace", known)).toEqual({
      kind: "participant_upsert",
      agentAlias: "mokosh",
      body: "stabilize the workspace",
    });
    expect(parseLeadingAgentDirective("@dazhbog propose the clearest next step", known)).toEqual({
      kind: "participant_upsert",
      agentAlias: "dazhbog",
      body: "propose the clearest next step",
    });
  });
});
