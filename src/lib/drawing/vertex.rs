use crate::*;
use lyon::tessellation::{self, geometry_builder::VertexConstructor};

#[derive(Copy, Clone, Debug)]
pub enum VertexType {
    Polygon = 0,
    Line = 1,
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub struct Vertex {
    pub position: [i16; 2],
    pub normal: [i16; 2],
    pub feature_id: u32,
}

// A very simple vertex constructor that only outputs the vertex position
pub struct LayerVertexCtor {
    pub tile_id: math::TileId,
    pub feature_id: u32,
    pub extent: f32,
    pub vertex_type: VertexType,
}

impl LayerVertexCtor {
    pub fn new(tile_id: &TileId, extent: f32) -> Self {
        Self {
            tile_id: *tile_id,
            feature_id: 0,
            extent,
            vertex_type: VertexType::Polygon,
        }
    }
}

impl VertexConstructor<tessellation::FillVertex, Vertex> for LayerVertexCtor {
    fn new_vertex(&mut self, vertex: tessellation::FillVertex) -> Vertex {
        assert!(!vertex.position.x.is_nan());
        assert!(!vertex.position.y.is_nan());
        const LIMIT: f32 = 3.0;
        let normal = if vertex.normal.length() > LIMIT {
            vertex.normal.normalize() * LIMIT
        } else {
            vertex.normal
        } * self.extent;

        let meta: u16 = self.vertex_type as u16;

        Vertex {
            position: [vertex.position.x as i16, vertex.position.y as i16],
            normal: [normal.x.round() as i16, normal.y.round() as i16],
            feature_id: ((meta as u32) << 16) | self.feature_id,
        }
    }
}

impl VertexConstructor<tessellation::StrokeVertex, Vertex> for LayerVertexCtor {
    fn new_vertex(&mut self, vertex: tessellation::StrokeVertex) -> Vertex {
        assert!(!vertex.position.x.is_nan());
        assert!(!vertex.position.y.is_nan());
        let normal = if vertex.normal.length() > 8.0 {
            vertex.normal.normalize() * 8.0
        } else {
            vertex.normal
        } * self.extent;

        Vertex {
            position: [vertex.position.x as i16, vertex.position.y as i16],
            normal: [normal.x.round() as i16, normal.y.round() as i16],
            feature_id: self.feature_id,
        }
    }
}
