// Tagged arrays preserve Rust object order, including integer-like keys that JS objects reorder.
export function ordered(value: any): any {
  if (typeof value === "string") return ["string", Array.from({ length: value.length }, (_, i) => value.charCodeAt(i))];
  if (Array.isArray(value)) return ["array", value.map(ordered)];
  if (value && typeof value === "object") return ["object", Object.entries(value).map(([key, v]) => [key, ordered(v)])];
  return ["scalar", value];
}
export function plain(node: any): any {
  if (node[0] === "string") return node[1].map((n: number) => String.fromCharCode(n)).join("");
  if (node[0] === "array") return node[1].map(plain);
  if (node[0] === "object") return Object.fromEntries(node[1].map(([key, value]: any) => [key, plain(value)]));
  return node[1];
}
export function normalizeOrdered(node: any, normalize: (s: string) => string): any {
  if (node[0] === "string") return ordered(normalize(plain(node)));
  if (node[0] === "array") return ["array", node[1].map((v: any) => normalizeOrdered(v, normalize))];
  if (node[0] === "object") return ["object", node[1].map(([key, value]: any) => [normalize(key), normalizeOrdered(value, normalize)])];
  return node;
}
