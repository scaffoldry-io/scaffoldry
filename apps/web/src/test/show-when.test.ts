import { describe, expect, it } from "vitest";
import vectors from "./vectors/show-when.json";
import { visibleFields, type FormFieldConditions } from "../showWhen";

type Vector = { name: string; fields: FormFieldConditions[]; answers: Record<string, unknown>; visible: { name: string; visible: boolean }[] };

// Views phase 7, test 1: every vector the Rust evaluator generated gives the same visibility here.
describe("show_when in the browser", () => {
  const list = (vectors as { vectors: Vector[] }).vectors;

  it("has vectors to check", () => {
    expect(list.length).toBeGreaterThan(10);
  });

  for (const v of list) {
    it(`agrees with the server: ${v.name}`, () => {
      const shown = visibleFields(v.fields, v.answers);
      expect(v.fields.map((f, i) => ({ name: f.name, visible: shown[i] }))).toEqual(v.visible);
    });
  }
});
