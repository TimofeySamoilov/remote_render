struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) world_position: vec3<f32>,
};

@vertex
fn vertex(
    @builtin(model) model_matrix: mat4x4<f32>,
    @builtin(normal) model_normal: vec3<f32>,
    @location(0) position: vec3<f32>,
) -> VertexOutput {
    let world_position: vec3<f32> = (model_matrix * vec4<f32>(position, 1.0)).xyz;
    let world_normal: vec3<f32> = normalize((model_matrix * vec4<f32>(model_normal, 0.0)).xyz);

    var out: VertexOutput;
    out.clip_position = camera.view_projection * vec4<f32>(world_position, 1.0);
    out.world_normal = world_normal;
    out.world_position = world_position;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Simple normal visualization: map normal components to RGB.
    // Multiply by 0.5 and add 0.5 to remap from [-1, 1] to [0, 1].
    return vec4<f32>(in.world_normal * 0.5 + 0.5, 1.0);
}