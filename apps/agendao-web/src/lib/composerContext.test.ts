import { describe, expect, it } from "vitest";
import { clipboardImageFiles } from "./composerContext";

function clipboardItem(kind: string, type: string, file: File | null): DataTransferItem {
  return {
    kind,
    type,
    getAsFile: () => file,
    getAsString: () => {
      throw new Error("not implemented");
    },
    webkitGetAsEntry: () => null,
  } as unknown as DataTransferItem;
}

describe("clipboardImageFiles", () => {
  it("collects image files from clipboard items", () => {
    const image = new File(["image"], "shot.png", { type: "image/png" });
    const text = new File(["text"], "note.txt", { type: "text/plain" });

    const files = clipboardImageFiles([
      clipboardItem("string", "text/plain", null),
      clipboardItem("file", "text/plain", text),
      clipboardItem("file", "image/png", image),
    ]);

    expect(files).toEqual([image]);
  });

  it("drops null file handles", () => {
    const files = clipboardImageFiles([
      clipboardItem("file", "image/png", null),
      clipboardItem("file", "image/jpeg", null),
    ]);

    expect(files).toEqual([]);
  });
});
