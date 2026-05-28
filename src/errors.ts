export class EnvswitchError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "EnvswitchError";
  }
}

export function fail(message: string): never {
  throw new EnvswitchError(message);
}
