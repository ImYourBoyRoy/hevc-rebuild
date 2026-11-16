// ./desktop/src/lib/state/bind-host.ts
// Attaches read-only members that live elsewhere (route deriveds, other state) to a state-class
// instance so the instance can serve as a controller host. Each getter is evaluated on every read,
// so reactivity is preserved inside `$derived` and effects.

export function withGetters<T extends object>(target: T, getters: { [K in keyof T]?: () => T[K] }): T {
  for (const [key, get] of Object.entries(getters)) {
    Object.defineProperty(target, key, { get: get as () => unknown, enumerable: true, configurable: true });
  }
  return target;
}

type UnionToIntersection<U> = (U extends unknown ? (value: U) => void : never) extends (value: infer I) => void ? I : never;

// Presents several objects (state-class instances plus an object of route-derived members and
// functions) as one host. Reads and writes go to the first part that has the key; earlier parts win,
// so route-specific members override shared defaults. The return type is the intersection of the
// parts, so assigning it to a controller's host interface is still checked by the compiler.
export function composeHost<T extends object[]>(...parts: T): UnionToIntersection<T[number]> {
  const find = (key: PropertyKey) => parts.find((part) => key in part);
  return new Proxy({}, {
    get(_target, key) {
      const part = find(key);
      return part ? Reflect.get(part, key, part) : undefined;
    },
    set(_target, key, value) {
      const part = find(key);
      if (!part) throw new TypeError(`composeHost: no part has a member named ${String(key)}`);
      return Reflect.set(part, key, value, part);
    },
    has(_target, key) {
      return Boolean(find(key));
    },
  }) as UnionToIntersection<T[number]>;
}
