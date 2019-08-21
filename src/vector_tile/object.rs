use lyon::math::Point;
use std::collections::HashMap;
use crate::css::Selector;

#[derive(Debug, Clone)]
pub enum ObjectType {
    Polygon,
    Line,
    Point,
}

#[derive(Debug, Clone)]
pub struct Object {
    pub selector: Selector,
    pub points: Vec<Point>,
    pub tags: HashMap<String, String>,
    pub object_type: ObjectType,
}

impl Object {
    pub fn new(
        selector: Selector,
        points: Vec<Point>,
        tags: HashMap<String, String>,
        object_type: ObjectType
    ) -> Self {
        Self {
            selector,
            points,
            tags: HashMap::new(),
            object_type
        }
    }
}