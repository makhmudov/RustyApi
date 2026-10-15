use rustyapi::prelude::*;
use rustyapi::schema::Components;

/// Статус задачи.
#[schema]
#[serde(rename_all = "snake_case")]
enum Status {
    InProgress,
    Done,
}

#[schema]
#[derive(PartialEq)]
struct Tag {
    #[field(min_length = 1)]
    name: String,
}

#[schema]
#[serde(rename_all = "camelCase")]
struct Item {
    #[field(min_length = 1, max_length = 100, example = "Молоток")]
    item_name: String,
    #[field(gt = 0, le = 1_000_000.5)]
    price: f64,
    #[field(ge = -10, lt = 10)]
    delta: Option<i32>,
    #[field(email)]
    owner_email: Option<String>,
    #[field(url)]
    homepage: Option<String>,
    #[field(pattern = r"^[a-z]+$", default = "abc")]
    code: String,
    #[field(default = Status::InProgress)]
    status: Status,
    #[field(max_length = 2)]
    tags: Vec<Tag>,
    #[serde(rename = "extra-data")]
    extra: Option<Value>,
    #[serde(skip)]
    cache: u8,
}

#[schema]
struct Page<T> {
    items: Vec<T>,
    total: u64,
}

fn main() {
    let item: Item = rustyapi::serde_json::from_value(json!({
        "itemName": "x", "price": 1, "tags": [{"name": ""}, {"name": "b"}, {"name": "c"}]
    }))
    .unwrap();
    assert_eq!(item.code, "abc");
    assert_eq!(item.cache, 0);
    let err = item.validate().unwrap_err();
    let types: Vec<_> = err.errors().iter().map(|e| e.type_.to_string()).collect();
    assert_eq!(types, ["too_long", "string_too_short"]);
    assert_eq!(err.errors()[1].loc, vec!["tags".into(), 0usize.into(), "name".into()]);

    let mut components = Components::new();
    let reference = Page::<Item>::schema(&mut components);
    assert_eq!(reference, json!({"$ref": "#/components/schemas/Page_Item_"}));
    let item_schema = components.get("Item").unwrap();
    assert_eq!(item_schema["required"], json!(["itemName", "price", "tags"]));
    assert_eq!(item_schema["properties"]["code"]["default"], json!("abc"));
    assert_eq!(item_schema["properties"]["price"]["exclusiveMinimum"], json!(0.0));
    assert!(item_schema["properties"].get("cache").is_none());
    assert!(components.get("Status").is_some());
    let _ = (Tag { name: "a".into() } == Tag { name: "a".into() }, Status::Done);
}
