// Shared fixture expansion is transport only: all envswitch behavior stays in the oracle.
export type ResponseFixture = { argv: string[]; stdout?: string; stderr?: string; exitCode?: number };
export function responsesFor(s: any): ResponseFixture[] {
  const responses: ResponseFixture[] = (s.op?.responses ?? []).map((r: any) => ({
    argv: r.argv, stdout: r.stdoutRepeat ? r.stdoutRepeat.value.repeat(r.stdoutRepeat.count) : r.stdout ?? "",
    stderr: r.stderr ?? "", exitCode: r.exitCode ?? 0,
  }));
  const add = (argv: string[], value: unknown) => {
    if (!responses.some(r => JSON.stringify(r.argv) === JSON.stringify(argv))) responses.push({ argv, stdout: JSON.stringify(value), stderr: "", exitCode: 0 });
  };
  add(["whoami", "--format", "json"], {});
  add(["vault", "list", "--format", "json"], s.op?.vaults ?? [{ id: "alpha", name: "alpha" }]);
  const vaults = new Set([s.vault ?? "alpha", ...Object.keys(s.op?.byVault ?? {}), ...(s.op?.vaults ?? []).map((v: any) => v.id || v.name)]);
  for (const vault of vaults) {
    const v = { ...s.op, ...s.op?.byVault?.[vault] };
    const list = ["item", "list", "--vault", vault, "--format", "json"];
    add([...list, "--tags", "envswitch-config"], v.configItems ?? [{ id: "cfg", title: "demo-suite", tags: ["envswitch-config"] }]);
    for (const tags of s.candidateTags ?? ["env:local", "env:dev", "envswitch,project:demo-suite,env:local", "envswitch,project:demo-suite,env:dev"]) add([...list, "--tags", tags], v.items ?? []);
    add(list, v.items ?? []);
    const configRaw = s.configRaw ?? JSON.stringify(("config" in s ? s.config : { targets: [{ name: "api", output: "apps/api/.env", groups: ["apis"] }] }));
    const details = { cfg: { fields: [{ label: "config", value: configRaw }] }, ...v.details };
    for (const [item, detail] of Object.entries(details)) add(["item", "get", item, "--vault", vault, "--format", "json", "--reveal"], detail);
    for (const item of v.items ?? []) {
      const id = item.id || item.title;
      add(["item", "get", id, "--vault", vault, "--format", "json", "--reveal"], { fields: Object.entries(v.values?.[id] ?? {}).map(([label, value]) => ({ label, value })) });
    }
  }
  return responses;
}
