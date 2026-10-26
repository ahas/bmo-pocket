import type { BarProperty } from "bmo-ui/app/domain/tabs";

// A value the host hands the screen, shown as what it is rather than as JSON (docs/mcp-host.md, 「How the screen
// reads」): an object as its named fields, a list as its rows by their place, each nested one under its parent's
// name; and a schema as its fields, one row each, with its type, whether it is required and what it takes.

type Translate = (key: string, values?: Record<string, unknown>) => string;

// The mark between a parent's name and its member's.
const UNDER = " › ";

function scalar(t: Translate, value: unknown): string {
  if (value === null || value === undefined) return t("pocket.status.none");
  if (typeof value === "boolean") return t(value ? "pocket.status.yes" : "pocket.status.no");

  return String(value);
}

/** A value's named fields, one row each, a nested field named under its parent's; none where it holds nothing. */
export function valueRows(t: Translate, value: unknown, under = ""): BarProperty[] {
  const named = (name: string) => (under === "" ? name : `${under}${UNDER}${name}`);

  if (Array.isArray(value)) return value.flatMap((one, at) => valueRows(t, one, named(String(at + 1))));

  if (value !== null && typeof value === "object") {
    return Object.entries(value).flatMap(([name, one]) => valueRows(t, one, named(name)));
  }

  return under === "" ? [] : [{ label: under, value: scalar(t, value), mono: typeof value === "string" }];
}

/** A yes, a no, or not known, as the state word it is. */
export function stateWord(t: Translate, value: boolean | null): string {
  return value === null ? t("pocket.status.unknown") : t(value ? "pocket.status.yes" : "pocket.status.no");
}

type Schema = Record<string, unknown>;

const asSchema = (value: unknown): Schema | null => (value !== null && typeof value === "object" && !Array.isArray(value) ? (value as Schema) : null);

// A schema's type in words: its type or types, an item's type for a list.
function typeOf(schema: Schema): string {
  const types = Array.isArray(schema.type) ? schema.type.map(String) : typeof schema.type === "string" ? [schema.type] : [];
  const items = asSchema(schema.items);
  const shown = types.map((type) => (type === "array" && items !== null && typeOf(items) !== "" ? `${type}<${typeOf(items)}>` : type));

  return shown.join(" | ");
}

// What a field takes beyond its type: its choices, its range, its length, its default.
function takes(t: Translate, schema: Schema): string[] {
  const said: string[] = [];

  if (Array.isArray(schema.enum)) said.push(t("pocket.schema.choices", { values: schema.enum.map((one) => scalar(t, one)).join(", ") }));
  if (schema.const !== undefined) said.push(t("pocket.schema.fixed", { value: scalar(t, schema.const) }));
  if (typeof schema.minimum === "number" || typeof schema.maximum === "number") {
    said.push(t("pocket.schema.range", { from: schema.minimum ?? "", to: schema.maximum ?? "" }));
  }
  if (typeof schema.minLength === "number" || typeof schema.maxLength === "number") {
    said.push(t("pocket.schema.length", { from: schema.minLength ?? "", to: schema.maxLength ?? "" }));
  }
  if (typeof schema.minItems === "number" || typeof schema.maxItems === "number") {
    said.push(t("pocket.schema.items", { from: schema.minItems ?? "", to: schema.maxItems ?? "" }));
  }
  if (schema.default !== undefined) said.push(t("pocket.schema.default", { value: scalar(t, schema.default) }));

  return said;
}

/** One field of a schema as the screen lays it out. */
export interface SchemaField {
  name: string;
  depth: number;
  type: string;
  required: boolean;
  description: string;
  takes: string[];
}

/** A schema's fields, one row each, a nested object's or a list item's fields under their parent; none where the
 * schema names no fields, which then stands as code. */
export function schemaFields(t: Translate, value: unknown, depth = 0): SchemaField[] {
  const schema = asSchema(value);
  const properties = asSchema(schema?.properties);

  if (schema === null || properties === null) return [];

  const required = new Set(Array.isArray(schema.required) ? schema.required.map(String) : []);

  return Object.entries(properties).flatMap(([name, raw]) => {
    const field = asSchema(raw) ?? {};
    const items = asSchema(field.items);
    const nested = field.properties !== undefined ? field : items?.properties !== undefined ? items : null;

    return [
      {
        name,
        depth,
        type: typeOf(field),
        required: required.has(name),
        description: typeof field.description === "string" ? field.description : "",
        takes: takes(t, field)
      },
      ...(nested === null ? [] : schemaFields(t, nested, depth + 1))
    ];
  });
}
