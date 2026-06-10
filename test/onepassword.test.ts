import { describe, expect, test } from "bun:test";
import { OnePasswordClient, type CommandRunner } from "../src/onepassword.ts";

describe("onepassword", () => {
  test("lists vault summaries", async () => {
    await expect(OnePasswordClient.listVaults(new StaticRunner({
      "vault list": JSON.stringify([{ id: "vault-a", name: "Vault A" }]),
    }))).resolves.toEqual([{ id: "vault-a", name: "Vault A" }]);
  });

  test("rejects non-array item list output", async () => {
    const op = new OnePasswordClient("vault", new StaticRunner({ "item list": "{}" }));

    await expect(op.listItemsByTags(["envswitch"])).rejects.toThrow("op item list returned non-array JSON");
  });

  test("rejects invalid item JSON", async () => {
    const op = new OnePasswordClient("vault", new StaticRunner({ "item list": "not-json" }));

    await expect(op.listItemsByTags([])).rejects.toThrow("op item list output is not valid JSON");
  });

  test("requires a non-empty config field", async () => {
    const op = new OnePasswordClient("vault", new StaticRunner({
      "item get cfg": JSON.stringify({ fields: [{ label: "config", value: "" }] }),
    }));

    await expect(op.readConfigItem("cfg")).rejects.toThrow("must contain a non-empty field named config");
  });

  test("reads only valid env field labels", async () => {
    const op = new OnePasswordClient("vault", new StaticRunner({
      "item get env": JSON.stringify({
        fields: [
          { label: "VALID_KEY", value: "1" },
          { label: "invalid-key", value: "2" },
          { label: "notesPlain", value: "ignored" },
        ],
      }),
    }));

    await expect(op.readEnvItem({ id: "env", title: "env" })).resolves.toEqual({ VALID_KEY: "1" });
  });
});

class StaticRunner implements CommandRunner {
  constructor(private readonly responses: Record<string, string>) {}

  async run(_command: string, args: string[]): Promise<string> {
    if (args[0] === "vault" && args[1] === "list") {
      return this.responses["vault list"] ?? "[]";
    }
    if (args[0] === "item" && args[1] === "list") {
      return this.responses["item list"] ?? "[]";
    }
    if (args[0] === "item" && args[1] === "get" && args[2]) {
      return this.responses[`item get ${args[2]}`] ?? JSON.stringify({ fields: [] });
    }
    return "{}";
  }
}
