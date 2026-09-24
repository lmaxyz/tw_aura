use std::sync::{Arc, Mutex};

use egui::{PaintCallback, Rect, Vec2};
use egui_glow::CallbackFn;
use egui_rotate::Rotation;
use glow::HasContext;

use crate::twitch::stream::YuvFrame;

/// GPU YUV renderer using custom OpenGL shaders via `egui::PaintCallback`.
///
/// Uploads Y, U, V planes as single-channel textures and draws a quad with
/// a fragment shader that performs YUV→RGB conversion on the GPU.
/// This eliminates the FFmpeg software scaler and halves upload bandwidth.
pub struct YuvRenderer {
    inner: Arc<Mutex<YuvRendererInner>>,
}

struct YuvRendererInner {
    rotation: Option<Rotation>,
    pending: Option<YuvFrame>,

    // Lazily initialized GL resources
    program: Option<glow::NativeProgram>,
    vao: Option<glow::NativeVertexArray>,
    vbo: Option<glow::NativeBuffer>,
    y_tex: Option<glow::NativeTexture>,
    u_tex: Option<glow::NativeTexture>,
    v_tex: Option<glow::NativeTexture>,

    // `true` if we must use `GL_LUMINANCE` (GLES 2.0), otherwise `GL_RED`/`R8`.
    use_luminance: Option<bool>,
}

impl Default for YuvRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl YuvRenderer {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(YuvRendererInner {
                rotation: None,
                pending: None,
                program: None,
                vao: None,
                vbo: None,
                y_tex: None,
                u_tex: None,
                v_tex: None,
                use_luminance: None,
            })),
        }
    }

    /// Queue a new decoded frame for upload on the next paint.
    pub fn set_frame(&self, frame: YuvFrame) {
        self.inner.lock().unwrap().pending = Some(frame);
    }

    /// Set the current screen rotation (Aurora OS). `None` on desktop.
    /// Build an `egui::PaintCallback` that renders into `rect`.
    pub fn set_rotation(&self, rotation: Option<Rotation>) {
        self.inner.lock().unwrap().rotation = rotation;
    }

    pub fn paint_callback(&self, rect: Rect) -> PaintCallback {
        let inner = self.inner.clone();
        PaintCallback {
            rect,
            callback: Arc::new(CallbackFn::new(move |info, painter| {
                let gl = painter.gl();
                let mut state = inner.lock().unwrap();
                state.ensure_initialized(gl);
                state.upload_and_draw(gl, &info);
            })),
        }
    }
}

impl YuvRendererInner {
    fn ensure_initialized(&mut self, gl: &glow::Context) {
        if self.program.is_some() {
            return;
        }

        let version = unsafe { gl.get_parameter_string(glow::VERSION) };
        let is_gles = version.starts_with("OpenGL ES");
        let es_major = if is_gles {
            version
                .chars()
                .nth("OpenGL ES ".len())
                .and_then(|c| c.to_digit(10))
                .unwrap_or(2)
        } else {
            0
        };

        self.use_luminance = Some(is_gles && es_major < 3);

        let (vs_source, fs_source) = if is_gles && es_major < 3 {
            // GLES 2.0
            (
                r"#version 100
                attribute vec2 a_pos;
                attribute vec2 a_uv;
                varying vec2 v_uv;
                void main() {
                    gl_Position = vec4(a_pos, 0.0, 1.0);
                    v_uv = a_uv;
                }",
                r"#version 100
                precision mediump float;
                varying vec2 v_uv;
                uniform sampler2D y_tex;
                uniform sampler2D u_tex;
                uniform sampler2D v_tex;
                void main() {
                    float y = texture2D(y_tex, v_uv).r;
                    float u = texture2D(u_tex, v_uv).r - 0.5;
                    float v = texture2D(v_tex, v_uv).r - 0.5;
                    float r = y + 1.403 * v;
                    float g = y - 0.344 * u - 0.714 * v;
                    float b = y + 1.772 * u;
                    gl_FragColor = vec4(r, g, b, 1.0);
                }",
            )
        } else if is_gles {
            // GLES 3.0+
            (
                r"#version 300 es
                in vec2 a_pos;
                in vec2 a_uv;
                out vec2 v_uv;
                void main() {
                    gl_Position = vec4(a_pos, 0.0, 1.0);
                    v_uv = a_uv;
                }",
                r"#version 300 es
                precision mediump float;
                in vec2 v_uv;
                out vec4 o_color;
                uniform sampler2D y_tex;
                uniform sampler2D u_tex;
                uniform sampler2D v_tex;
                void main() {
                    float y = texture(y_tex, v_uv).r;
                    float u = texture(u_tex, v_uv).r - 0.5;
                    float v = texture(v_tex, v_uv).r - 0.5;
                    float r = y + 1.403 * v;
                    float g = y - 0.344 * u - 0.714 * v;
                    float b = y + 1.772 * u;
                    o_color = vec4(r, g, b, 1.0);
                }",
            )
        } else {
            // Desktop GL
            (
                r"#version 330 core
                in vec2 a_pos;
                in vec2 a_uv;
                out vec2 v_uv;
                void main() {
                    gl_Position = vec4(a_pos, 0.0, 1.0);
                    v_uv = a_uv;
                }",
                r"#version 330 core
                in vec2 v_uv;
                out vec4 o_color;
                uniform sampler2D y_tex;
                uniform sampler2D u_tex;
                uniform sampler2D v_tex;
                void main() {
                    float y = texture(y_tex, v_uv).r;
                    float u = texture(u_tex, v_uv).r - 0.5;
                    float v = texture(v_tex, v_uv).r - 0.5;
                    float r = y + 1.403 * v;
                    float g = y - 0.344 * u - 0.714 * v;
                    float b = y + 1.772 * u;
                    o_color = vec4(r, g, b, 1.0);
                }",
            )
        };

        let program = unsafe {
            let vs = gl.create_shader(glow::VERTEX_SHADER).unwrap();
            gl.shader_source(vs, vs_source);
            gl.compile_shader(vs);
            if !gl.get_shader_compile_status(vs) {
                let info_log = gl.get_shader_info_log(vs);
                log::error!("YUV vertex shader compile error: {info_log}");
            }

            let fs = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
            gl.shader_source(fs, fs_source);
            gl.compile_shader(fs);
            if !gl.get_shader_compile_status(fs) {
                let info_log = gl.get_shader_info_log(fs);
                log::error!("YUV fragment shader compile error: {info_log}");
            }

            let prog = gl.create_program().unwrap();
            gl.attach_shader(prog, vs);
            gl.attach_shader(prog, fs);
            gl.link_program(prog);
            if !gl.get_program_link_status(prog) {
                let info_log = gl.get_program_info_log(prog);
                log::error!("YUV program link error: {info_log}");
            }

            gl.detach_shader(prog, vs);
            gl.detach_shader(prog, fs);
            gl.delete_shader(vs);
            gl.delete_shader(fs);

            prog
        };

        let vao = unsafe { gl.create_vertex_array().unwrap() };
        let vbo = unsafe { gl.create_buffer().unwrap() };

        unsafe {
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));

            let stride = (4 * std::mem::size_of::<f32>()) as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(
                1,
                2,
                glow::FLOAT,
                false,
                stride,
                2 * std::mem::size_of::<f32>() as i32,
            );

            gl.bind_vertex_array(None);
        }

        let y_tex = unsafe { gl.create_texture().unwrap() };
        let u_tex = unsafe { gl.create_texture().unwrap() };
        let v_tex = unsafe { gl.create_texture().unwrap() };

        // Initialize textures to 1×1 black so the first paint doesn't show garbage.
        let black: [u8; 1] = [0];
        let use_luminance = self.use_luminance.unwrap();
        self.upload_plane(gl, y_tex, 1, 1, &black, use_luminance);
        self.upload_plane(gl, u_tex, 1, 1, &black, use_luminance);
        self.upload_plane(gl, v_tex, 1, 1, &black, use_luminance);

        self.program = Some(program);
        self.vao = Some(vao);
        self.vbo = Some(vbo);
        self.y_tex = Some(y_tex);
        self.u_tex = Some(u_tex);
        self.v_tex = Some(v_tex);
    }

    fn upload_and_draw(&mut self, gl: &glow::Context, info: &egui::PaintCallbackInfo) {
        let program = self.program.unwrap();
        let vao = self.vao.unwrap();
        let y_tex = self.y_tex.unwrap();
        let u_tex = self.u_tex.unwrap();
        let v_tex = self.v_tex.unwrap();
        let use_luminance = self.use_luminance.unwrap();

        unsafe {
            gl.use_program(Some(program));
        }

        // `egui-rotate` transforms clip_rect back to physical space during tessellation,
        // but PaintCallback::rect (info.viewport) stays in logical space. We must manually
        // transform the widget rect to physical space so the viewport matches where the
        // video actually appears on screen. Use the *widget rect* (`info.viewport`), not
        // `clip_rect` (which is the parent's clip region and can be much larger).
        let viewport_px = match self.rotation {
            None | Some(Rotation::None) => info.viewport_in_pixels(),
            Some(rotation) => {
                // logical_size must be the FULL LOGICAL SCREEN size, not the widget size.
                // inverse_transform_pos needs the canvas dimensions to map correctly.
                let logical_size = if rotation.swaps_axes() {
                    Vec2::new(
                        info.screen_size_px[1] as f32 / info.pixels_per_point,
                        info.screen_size_px[0] as f32 / info.pixels_per_point,
                    )
                } else {
                    Vec2::new(
                        info.screen_size_px[0] as f32 / info.pixels_per_point,
                        info.screen_size_px[1] as f32 / info.pixels_per_point,
                    )
                };
                let physical_min = rotation.inverse_transform_pos(info.viewport.min, logical_size);
                let physical_max = rotation.inverse_transform_pos(info.viewport.max, logical_size);
                let physical_rect = Rect::from_two_pos(physical_min, physical_max);
                egui::epaint::ViewportInPixels::from_points(
                    &physical_rect,
                    info.pixels_per_point,
                    info.screen_size_px,
                )
            }
        };
        unsafe {
            gl.viewport(
                viewport_px.left_px,
                viewport_px.from_bottom_px,
                viewport_px.width_px,
                viewport_px.height_px,
            );
        }

        // Upload new frame if available
        if let Some(frame) = self.pending.take() {
            self.upload_plane(
                gl,
                y_tex,
                frame.width,
                frame.height,
                &frame.y,
                use_luminance,
            );
            self.upload_plane(
                gl,
                u_tex,
                frame.width / 2,
                frame.height / 2,
                &frame.u,
                use_luminance,
            );
            self.upload_plane(
                gl,
                v_tex,
                frame.width / 2,
                frame.height / 2,
                &frame.v,
                use_luminance,
            );
        }

        // Build a full-screen NDC quad. The viewport override places it in the correct
        // physical area; no UV rotation is needed.
        let vertices = self.quad_vertices();

        let vertices_bytes = unsafe {
            std::slice::from_raw_parts(
                vertices.as_ptr() as *const u8,
                vertices.len() * std::mem::size_of::<f32>(),
            )
        };

        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, self.vbo);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertices_bytes, glow::DYNAMIC_DRAW);

            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(y_tex));
            gl.uniform_1_i32(gl.get_uniform_location(program, "y_tex").as_ref(), 0);

            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(u_tex));
            gl.uniform_1_i32(gl.get_uniform_location(program, "u_tex").as_ref(), 1);

            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_2D, Some(v_tex));
            gl.uniform_1_i32(gl.get_uniform_location(program, "v_tex").as_ref(), 2);

            gl.bind_vertex_array(Some(vao));
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.bind_vertex_array(None);
            gl.use_program(None);

            // Restore full-window viewport so subsequent egui meshes render correctly.
            gl.viewport(
                0,
                0,
                info.screen_size_px[0] as i32,
                info.screen_size_px[1] as i32,
            );

            // Be kind to the egui_glow painter: reset texture state.
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }

    /// Returns a 16-element f32 array: [pos_x, pos_y, uv_u, uv_v] × 4 vertices.
    ///
    /// Positions are fixed (full-screen NDC quad as a triangle strip).
    /// UVs are rotated to match the egui-rotate transform so that the video
    /// appears upright after the compositor rotates the framebuffer.
    fn quad_vertices(&self) -> [f32; 16] {
        match self.rotation {
            Some(Rotation::CW270) => [
                -1.0, -1.0, 1.0, 1.0, // bottom-left → texture bottom-right
                1.0, -1.0, 1.0, 0.0, // bottom-right → texture top-right
                -1.0, 1.0, 0.0, 1.0, // top-left → texture bottom-left
                1.0, 1.0, 0.0, 0.0, // top-right → texture top-left
            ],
            Some(Rotation::CW180) => [
                -1.0, -1.0, 1.0, 0.0, // bottom-left → texture top-right
                1.0, -1.0, 0.0, 0.0, // bottom-right → texture top-left
                -1.0, 1.0, 1.0, 1.0, // top-left → texture bottom-right
                1.0, 1.0, 0.0, 1.0, // top-right → texture bottom-left
            ],
            Some(Rotation::CW90) => [
                -1.0, -1.0, 0.0, 0.0, // bottom-left → texture top-left
                1.0, -1.0, 0.0, 1.0, // bottom-right → texture bottom-left
                -1.0, 1.0, 1.0, 0.0, // top-left → texture top-right
                1.0, 1.0, 1.0, 1.0, // top-right → texture bottom-right
            ],
            _ => [
                -1.0, -1.0, 0.0, 1.0, // bottom-left
                1.0, -1.0, 1.0, 1.0, // bottom-right
                -1.0, 1.0, 0.0, 0.0, // top-left
                1.0, 1.0, 1.0, 0.0, // top-right
            ],
        }
    }

    fn upload_plane(
        &self,
        gl: &glow::Context,
        tex: glow::NativeTexture,
        w: u32,
        h: u32,
        data: &[u8],
        use_luminance: bool,
    ) {
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );

            if use_luminance {
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::LUMINANCE as i32,
                    w as i32,
                    h as i32,
                    0,
                    glow::LUMINANCE,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(data)),
                );
            } else {
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::R8 as i32,
                    w as i32,
                    h as i32,
                    0,
                    glow::RED,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(data)),
                );
            }
        }
    }
}
