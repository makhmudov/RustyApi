//! `tez generate client --ts`: TypeScript-клиент из OpenAPI приложения.
//!
//! Интерфейсы для `components.schemas` и класс `Client` с методом на каждую операцию (имя —
//! имя обработчика в camelCase). Работает на `fetch` (браузер, Node 18+, Deno, Bun).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use serde_json::{Map, Value};

const RUNTIME: &str = r#"export class ApiError extends Error {
  readonly status: number;
  readonly detail: unknown;

  constructor(status: number, detail: unknown) {
    super(`HTTP ${status}: ${typeof detail === "string" ? detail : JSON.stringify(detail)}`);
    this.name = "ApiError";
    this.status = status;
    this.detail = detail;
  }
}

export interface ClientOptions {
  /** Адрес API, например `http://127.0.0.1:8000`. */
  baseUrl?: string;
  /** Токен для `Authorization: Bearer ...`. */
  token?: string | null;
  /** Свой `fetch` (по умолчанию глобальный). */
  fetch?: typeof fetch;
  /** Заголовки для всех запросов. */
  headers?: Record<string, string>;
}

type QueryValue = string | number | boolean | null | undefined | Array<string | number | boolean>;

interface RequestInit_ {
  query?: Record<string, QueryValue>;
  json?: unknown;
  form?: object;
  formData?: FormData;
}

export class Client {
  baseUrl: string;
  token: string | null;
  private readonly fetchImpl: typeof fetch;
  private readonly headers: Record<string, string>;

  constructor(options: ClientOptions = {}) {
    this.baseUrl = (options.baseUrl ?? "").replace(/\/$/, "");
    this.token = options.token ?? null;
    this.fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
    this.headers = options.headers ?? {};
  }

  /** Токен для следующих запросов (`null` — выйти). */
  setToken(token: string | null): void {
    this.token = token;
  }

  private async request<T>(method: string, path: string, init: RequestInit_ = {}): Promise<T> {
    let url = this.baseUrl + path;
    if (init.query) {
      const search = new URLSearchParams();
      for (const [key, value] of Object.entries(init.query)) {
        if (value === undefined || value === null) continue;
        if (Array.isArray(value)) value.forEach((item) => search.append(key, String(item)));
        else search.append(key, String(value));
      }
      const encoded = search.toString();
      if (encoded) url += "?" + encoded;
    }
    const headers: Record<string, string> = { ...this.headers };
    if (this.token) headers["Authorization"] = `Bearer ${this.token}`;
    let body: BodyInit | undefined;
    if (init.json !== undefined) {
      headers["Content-Type"] = "application/json";
      body = JSON.stringify(init.json);
    } else if (init.form !== undefined) {
      const form = new URLSearchParams();
      for (const [key, value] of Object.entries(init.form)) {
        if (value !== undefined && value !== null) form.append(key, String(value));
      }
      body = form;
    } else if (init.formData !== undefined) {
      body = init.formData;
    }
    const response = await this.fetchImpl(url, { method, headers, body });
    const text = await response.text();
    const isJson = (response.headers.get("content-type") ?? "").includes("application/json");
    const data: unknown = text && isJson ? JSON.parse(text) : text;
    if (!response.ok) {
      const detail =
        data !== null && typeof data === "object" && "detail" in data
          ? (data as { detail: unknown }).detail
          : data;
      throw new ApiError(response.status, detail);
    }
    return (text ? data : undefined) as T;
  }
"#;

const RESERVED: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "let",
    "static",
    "yield",
    "await",
];

/// Генерирует файл клиента.
pub fn generate(openapi: &Value) -> String {
    let mut out = String::new();
    let title = openapi["info"]["title"].as_str().unwrap_or("API");
    let version = openapi["info"]["version"].as_str().unwrap_or("");
    let _ = writeln!(
        out,
        "// Клиент {title} {version}. Сгенерировано `tez generate client --ts`, не редактируйте вручную.\n"
    );
    out.push_str("/* eslint-disable */\n\n");

    if let Some(schemas) = openapi["components"]["schemas"].as_object() {
        for (name, schema) in schemas {
            out.push_str(&type_declaration(&type_name(name), schema));
            out.push('\n');
        }
    }

    out.push_str(RUNTIME);
    let mut used = BTreeSet::new();
    if let Some(paths) = openapi["paths"].as_object() {
        for (path, item) in paths {
            let Some(item) = item.as_object() else {
                continue;
            };
            for (method, operation) in item {
                if !["get", "post", "put", "patch", "delete", "head", "options"]
                    .contains(&method.as_str())
                {
                    continue;
                }
                out.push('\n');
                out.push_str(&operation_method(path, method, operation, &mut used));
            }
        }
    }
    out.push_str("}\n");
    out
}

/// Имя типа TypeScript из имени схемы.
fn type_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn ref_name(reference: &str) -> String {
    type_name(reference.rsplit('/').next().unwrap_or(reference))
}

fn doc_comment(schema: &Value, indent: &str) -> String {
    let text = schema["description"]
        .as_str()
        .or_else(|| schema["summary"].as_str());
    match text {
        Some(text) if !text.trim().is_empty() => {
            let lines: Vec<&str> = text.trim().lines().collect();
            if lines.len() == 1 {
                format!("{indent}/** {} */\n", lines[0].replace("*/", "*\\/"))
            } else {
                let mut out = format!("{indent}/**\n");
                for line in lines {
                    let _ = writeln!(out, "{indent} * {}", line.replace("*/", "*\\/"));
                }
                out.push_str(&format!("{indent} */\n"));
                out
            }
        }
        _ => String::new(),
    }
}

fn type_declaration(name: &str, schema: &Value) -> String {
    let doc = doc_comment(schema, "");
    let is_object = schema["type"] == "object" && schema.get("properties").is_some();
    if is_object {
        format!("{doc}export interface {name} {}\n", object_body(schema, ""))
    } else {
        format!("{doc}export type {name} = {};\n", ts_type(schema))
    }
}

fn property_key(key: &str) -> String {
    let valid = key
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if valid {
        key.to_owned()
    } else {
        serde_json::to_string(key).unwrap_or_default()
    }
}

/// `{ a: T; b?: U }` (многострочно с отступом `indent`).
fn object_body(schema: &Value, indent: &str) -> String {
    let required: BTreeSet<&str> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let empty = Map::new();
    let properties = schema["properties"].as_object().unwrap_or(&empty);
    if properties.is_empty() {
        return "{}".to_owned();
    }
    let mut out = String::from("{\n");
    let inner = format!("{indent}  ");
    for (key, prop) in properties {
        out.push_str(&doc_comment(prop, &inner));
        let optional = if required.contains(key.as_str()) {
            ""
        } else {
            "?"
        };
        let _ = writeln!(
            out,
            "{inner}{}{optional}: {};",
            property_key(key),
            ts_type(prop)
        );
    }
    out.push_str(indent);
    out.push('}');
    out
}

fn union(mut parts: Vec<String>) -> String {
    let mut seen = BTreeSet::new();
    parts.retain(|p| seen.insert(p.clone()));
    if parts.iter().any(|p| p == "unknown") {
        return "unknown".to_owned();
    }
    match parts.len() {
        0 => "never".to_owned(),
        1 => parts.remove(0),
        _ => parts.join(" | "),
    }
}

/// Тип TypeScript для JSON Schema.
pub fn ts_type(schema: &Value) -> String {
    let Some(object) = schema.as_object() else {
        return if schema == &Value::Bool(false) {
            "never".to_owned()
        } else {
            "unknown".to_owned()
        };
    };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        return ref_name(reference);
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(variants) = object.get(key).and_then(Value::as_array) {
            return union(variants.iter().map(ts_type).collect());
        }
    }
    if let Some(all) = object.get("allOf").and_then(Value::as_array) {
        let parts: Vec<String> = all.iter().map(ts_type).collect();
        return if parts.len() == 1 {
            parts[0].clone()
        } else {
            parts.join(" & ")
        };
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        return union(values.iter().map(Value::to_string).collect());
    }
    if let Some(value) = object.get("const") {
        return value.to_string();
    }
    match object.get("type") {
        Some(Value::Array(types)) => union(
            types
                .iter()
                .map(|t| {
                    let mut single = object.clone();
                    single.insert("type".into(), t.clone());
                    ts_type(&Value::Object(single))
                })
                .collect(),
        ),
        Some(Value::String(t)) => match t.as_str() {
            "string" if object.get("format").and_then(Value::as_str) == Some("binary") => {
                "Blob".to_owned()
            }
            "string" => "string".to_owned(),
            "integer" | "number" => "number".to_owned(),
            "boolean" => "boolean".to_owned(),
            "null" => "null".to_owned(),
            "array" => {
                let item = object
                    .get("items")
                    .map_or_else(|| "unknown".to_owned(), ts_type);
                if item.contains(' ') {
                    format!("Array<{item}>")
                } else {
                    format!("{item}[]")
                }
            }
            "object" => {
                if object
                    .get("properties")
                    .is_some_and(|p| p.as_object().is_some_and(|p| !p.is_empty()))
                {
                    object_body(schema, "")
                } else {
                    match object.get("additionalProperties") {
                        Some(Value::Object(inner)) if !inner.is_empty() => {
                            format!("Record<string, {}>", ts_type(&Value::Object(inner.clone())))
                        }
                        _ => "Record<string, unknown>".to_owned(),
                    }
                }
            }
            _ => "unknown".to_owned(),
        },
        _ => "unknown".to_owned(),
    }
}

/// `read_todo` → `readTodo`.
fn camel_case(name: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in name.chars() {
        if c == '_' || c == '-' || c == ' ' {
            upper = !out.is_empty();
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

fn identifier(name: &str) -> String {
    let id = camel_case(name);
    if RESERVED.contains(&id.as_str()) {
        format!("{id}_")
    } else {
        id
    }
}

/// Имя обработчика из operationId в стиле FastAPI (`read_todo_todos__todo_id__get` → `read_todo`).
fn handler_name(operation_id: &str, path: &str, method: &str) -> String {
    let path_part: String = path
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let suffix = format!("{path_part}_{method}");
    operation_id
        .strip_suffix(&suffix)
        .filter(|s| !s.is_empty())
        .unwrap_or(operation_id)
        .to_owned()
}

fn operation_method(
    path: &str,
    method: &str,
    operation: &Value,
    used: &mut BTreeSet<String>,
) -> String {
    let operation_id = operation["operationId"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{method}{path}"));
    let mut name = camel_case(&handler_name(&operation_id, path, method));
    if !used.insert(name.clone()) {
        name = camel_case(&operation_id);
        used.insert(name.clone());
    }

    let parameters: Vec<&Value> = operation["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let mut args = Vec::new();
    let mut ts_path = path.to_owned();
    for param in parameters.iter().filter(|p| p["in"] == "path") {
        let raw = param["name"].as_str().unwrap_or_default();
        let arg = identifier(raw);
        args.push(format!("{arg}: {}", ts_type(&param["schema"])));
        ts_path = ts_path.replace(
            &format!("{{{raw}}}"),
            &format!("${{encodeURIComponent(String({arg}))}}"),
        );
    }
    let path_expr = if ts_path.contains("${") {
        format!("`{ts_path}`")
    } else {
        serde_json::to_string(&ts_path).unwrap_or_default()
    };

    let mut init = Vec::new();
    let mut body_arg = None;
    if let Some(content) = operation["requestBody"]["content"].as_object() {
        let required = operation["requestBody"]["required"] == true;
        let optional = if required { "" } else { "?" };
        if let Some(json) = content.get("application/json") {
            body_arg = Some(format!("body{optional}: {}", ts_type(&json["schema"])));
            init.push("json: body".to_owned());
        } else if let Some(form) = content.get("application/x-www-form-urlencoded") {
            body_arg = Some(format!("form{optional}: {}", ts_type(&form["schema"])));
            init.push("form".to_owned());
        } else if content.contains_key("multipart/form-data") {
            body_arg = Some(format!("formData{optional}: FormData"));
            init.push("formData".to_owned());
        }
    }
    args.extend(body_arg);

    let query: Vec<&&Value> = parameters.iter().filter(|p| p["in"] == "query").collect();
    if !query.is_empty() {
        let any_required = query.iter().any(|p| p["required"] == true);
        let fields: Vec<String> = query
            .iter()
            .map(|p| {
                let key = property_key(p["name"].as_str().unwrap_or_default());
                let optional = if p["required"] == true { "" } else { "?" };
                format!("{key}{optional}: {}", ts_type(&p["schema"]))
            })
            .collect();
        let default = if any_required { "" } else { " = {}" };
        args.push(format!("params: {{ {} }}{default}", fields.join("; ")));
        init.push("query: params".to_owned());
    }

    let response = response_type(operation);
    let init = if init.is_empty() {
        String::new()
    } else {
        format!(", {{ {} }}", init.join(", "))
    };
    let mut out = String::new();
    let doc = match (
        operation["summary"].as_str(),
        operation["description"].as_str(),
    ) {
        (Some(summary), Some(description)) => {
            serde_json::json!({ "description": format!("{summary}\n\n{description}") })
        }
        (Some(summary), None) => serde_json::json!({ "description": summary }),
        (None, Some(description)) => serde_json::json!({ "description": description }),
        (None, None) => Value::Null,
    };
    let mut doc = doc_comment(&doc, "  ");
    if doc.is_empty() {
        doc = format!("  /** `{} {path}` */\n", method.to_uppercase());
    }
    out.push_str(&doc);
    let _ = writeln!(
        out,
        "  async {name}({}): Promise<{response}> {{\n    return this.request<{response}>({:?}, {path_expr}{init});\n  }}",
        args.join(", "),
        method.to_uppercase(),
    );
    out
}

fn response_type(operation: &Value) -> String {
    let Some(responses) = operation["responses"].as_object() else {
        return "unknown".to_owned();
    };
    let success = responses.iter().find(|(code, _)| code.starts_with('2'));
    let Some((code, response)) = success else {
        return "unknown".to_owned();
    };
    let Some(content) = response["content"].as_object() else {
        return if code == "204" {
            "void".to_owned()
        } else {
            "unknown".to_owned()
        };
    };
    if let Some(json) = content.get("application/json") {
        return ts_type(&json["schema"]);
    }
    if content.keys().any(|k| k.starts_with("text/")) {
        return "string".to_owned();
    }
    "unknown".to_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn types() {
        assert_eq!(ts_type(&json!({"type": "integer"})), "number");
        assert_eq!(
            ts_type(&json!({"anyOf": [{"type": "string"}, {"type": "null"}]})),
            "string | null"
        );
        assert_eq!(
            ts_type(&json!({"type": "array", "items": {"$ref": "#/components/schemas/Todo"}})),
            "Todo[]"
        );
        assert_eq!(
            ts_type(
                &json!({"type": "array", "items": {"anyOf": [{"type": "integer"}, {"type": "null"}]}})
            ),
            "Array<number | null>"
        );
        assert_eq!(
            ts_type(&json!({"enum": ["todo", "done"]})),
            "\"todo\" | \"done\""
        );
        assert_eq!(ts_type(&json!({})), "unknown");
        assert_eq!(
            ts_type(&json!({"type": "object", "additionalProperties": {"type": "integer"}})),
            "Record<string, number>"
        );
        assert_eq!(
            ts_type(&json!({"type": "string", "format": "binary"})),
            "Blob"
        );
        assert_eq!(
            handler_name("read_todo_todos__todo_id__get", "/todos/{todo_id}", "get"),
            "read_todo"
        );
        assert_eq!(handler_name("root__get", "/", "get"), "root");
        assert_eq!(camel_case("list_users"), "listUsers");
        assert_eq!(identifier("default"), "default_");
    }

    #[test]
    fn operation() {
        let mut used = BTreeSet::new();
        let op = json!({
            "summary": "Read Todo",
            "operationId": "read_todo_todos__todo_id__get",
            "parameters": [
                {"name": "todo_id", "in": "path", "required": true, "schema": {"type": "integer"}},
                {"name": "full", "in": "query", "required": false, "schema": {"type": "boolean"}}
            ],
            "responses": {"200": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/Todo"}}}}}
        });
        assert_eq!(
            operation_method("/todos/{todo_id}", "get", &op, &mut used),
            "  /** Read Todo */\n  async readTodo(todoId: number, params: { full?: boolean } = {}): Promise<Todo> {\n    return this.request<Todo>(\"GET\", `/todos/${encodeURIComponent(String(todoId))}`, { query: params });\n  }\n"
        );
    }
}
