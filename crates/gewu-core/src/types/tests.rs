use super::*;

#[test]
fn nested_dictionaries_expose_their_value_type() {
    let encoded = DataType::Dictionary(
        Box::new(DataType::Int8),
        Box::new(DataType::Dictionary(
            Box::new(DataType::Int32),
            Box::new(DataType::Int64),
        )),
    );
    assert_eq!(value_type(&encoded), &DataType::Int64);
    assert!(is_describable(&encoded));
}
