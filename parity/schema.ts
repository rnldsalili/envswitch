// Validate the JSON Schema keywords used by transcript.schema.json, without a new runtime dependency.
export function validate(value: any, schema: any, root = schema, path = "$", errors: string[] = []): string[] {
  const fail = (message: string) => errors.push(`${path}: ${message}`);
  if (schema.$ref) return validate(value, schema.$ref.split("/").slice(1).reduce((s: any, key: string) => s[key], root), root, path, errors);
  if (schema.allOf) for (const s of schema.allOf) validate(value, s, root, path, errors);
  if (schema.if && !validate(value, schema.if, root, path, []).length) validate(value, schema.then, root, path, errors);
  if (schema.anyOf && !schema.anyOf.some((s: any) => !validate(value, s, root, path, []).length)) fail("no anyOf alternative matches");
  if (schema.oneOf && schema.oneOf.filter((s: any) => !validate(value, s, root, path, []).length).length !== 1) fail("expected exactly one matching alternative");
  if ("const" in schema && JSON.stringify(value) !== JSON.stringify(schema.const)) fail(`expected ${JSON.stringify(schema.const)}`);
  if (schema.enum && !schema.enum.includes(value)) fail(`not in enum ${schema.enum}`);
  const type = value === null ? "null" : Array.isArray(value) ? "array" : typeof value;
  if (schema.type && !(Array.isArray(schema.type) ? schema.type : [schema.type]).some((t: string) => t === type || (t === "integer" && Number.isInteger(value)))) { fail(`expected type ${schema.type}, got ${type}`); return errors; }
  if (type === "object") {
    for (const key of schema.required ?? []) if (!Object.hasOwn(value, key)) fail(`missing ${key}`);
    for (const [key, v] of Object.entries(value)) {
      if (schema.properties && Object.hasOwn(schema.properties, key)) validate(v, schema.properties[key], root, `${path}.${key}`, errors);
      else if (schema.additionalProperties === false) fail(`unexpected property ${key}`);
      else if (typeof schema.additionalProperties === "object") validate(v, schema.additionalProperties, root, `${path}.${key}`, errors);
    }
  }
  if (type === "array") {
    if (schema.minItems !== undefined && value.length < schema.minItems) fail("too few items");
    if (schema.uniqueItems && new Set(value.map((v: any) => JSON.stringify(v))).size !== value.length) fail("duplicate items");
    if (schema.items) value.forEach((v: any, i: number) => validate(v, schema.items, root, `${path}[${i}]`, errors));
  }
  if (type === "number") {
    if (schema.minimum !== undefined && value < schema.minimum) fail("below minimum");
    if (schema.maximum !== undefined && value > schema.maximum) fail("above maximum");
  }
  if (type === "string") {
    if (schema.pattern && !new RegExp(schema.pattern).test(value)) fail("pattern mismatch");
    if (schema.contentEncoding === "base64" && Buffer.from(value, "base64").toString("base64") !== value) fail("invalid canonical base64");
  }
  return errors;
}
