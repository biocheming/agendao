import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { SlashCommandMenu } from "./SlashCommandMenu";
import type { CommandApiSpec } from "../../lib/command";

const ITEMS: CommandApiSpec[] = [
  {
    name: "review",
    description: "Review code changes",
    aliases: ["rv"],
    source: "Builtin",
  },
  {
    name: "release",
    description: "Cut a release",
    source: "Builtin",
  },
];

describe("SlashCommandMenu", () => {
  it("renders command names, aliases and descriptions, and selects on mouse down", () => {
    const onSelect = vi.fn<(command: CommandApiSpec) => void>();
    render(
      <SlashCommandMenu
        items={ITEMS}
        activeIndex={0}
        onActiveIndexChange={vi.fn<(index: number) => void>()}
        onSelect={onSelect}
      />,
    );

    expect(screen.getByTestId("slash-command-item-review")).toHaveTextContent("/review");
    expect(screen.getByTestId("slash-command-item-review")).toHaveTextContent("/rv");
    expect(screen.getByTestId("slash-command-item-review")).toHaveTextContent(
      "Review code changes",
    );
    expect(screen.getByTestId("slash-command-item-release")).toHaveTextContent("Cut a release");

    fireEvent.mouseDown(screen.getByTestId("slash-command-item-release"));
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith(ITEMS[1]);
  });

  it("marks the active option and reports hover as active-index changes", () => {
    const onActiveIndexChange = vi.fn<(index: number) => void>();
    render(
      <SlashCommandMenu
        items={ITEMS}
        activeIndex={1}
        onActiveIndexChange={onActiveIndexChange}
        onSelect={vi.fn<(command: CommandApiSpec) => void>()}
      />,
    );

    expect(screen.getByTestId("slash-command-item-review")).toHaveAttribute(
      "data-active",
      "false",
    );
    expect(screen.getByTestId("slash-command-item-release")).toHaveAttribute(
      "data-active",
      "true",
    );
    expect(screen.getByTestId("slash-command-item-release")).toHaveAttribute(
      "aria-selected",
      "true",
    );

    fireEvent.mouseEnter(screen.getByTestId("slash-command-item-review"));
    expect(onActiveIndexChange).toHaveBeenCalledWith(0);
  });
});
