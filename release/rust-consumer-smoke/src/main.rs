use vsh::{VERSION, engine_kind};

fn main() {
    assert!(!VERSION.is_empty());
    assert_eq!(engine_kind(), "rust");
}
