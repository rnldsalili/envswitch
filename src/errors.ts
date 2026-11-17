export class EnvswitchError extends Error {
  /** `options.cause` carries diagnostics; only `message` is shown to users. */
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "EnvswitchError";
  }
}

export function fail(message: string): never {
  throw new EnvswitchError(message);
}
