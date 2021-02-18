#[allow(dead_code)]
pub enum ShaderStage {
    Vertex,
    Fragment,
    Compute,
}

// TODO lifetime issues
// only copy pasted for the time being...

// pub fn load_glsl<'a>(code: &'a str, stage: ShaderStage) -> wgpu::ShaderSource<'a> {
//     let ty = match stage {
//         ShaderStage::Vertex => shaderc::ShaderKind::Vertex,
//         ShaderStage::Fragment => shaderc::ShaderKind::Fragment,
//         ShaderStage::Compute => shaderc::ShaderKind::Compute,
//     };

//     let mut compiler = shaderc::Compiler::new().unwrap();
//     let binary_result = compiler
//         .compile_into_spirv(code, ty, "shader.glsl", "main", None)
//         .unwrap();
//     let binary_result = binary_result.as_binary_u8();
//     wgpu::util::make_spirv(&binary_result)
// }
