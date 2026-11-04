//! Shader scenes: Shadertoy-style GLSL (`mainImage(out vec4, in vec2)`) drawn
//! in a `GtkGLArea`. Uniforms: iTime, iTimeDelta, iFrame, iResolution, iMouse
//! (always zero), and audio: iSpectrum[32], iLevel, iBass (all 0..1).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use glow::HasContext;
use gtk::glib;
use gtk::prelude::*;

use crate::audio::{Audio, BANDS};

pub struct Builtin {
    pub key: &'static str,
    pub title: &'static str,
    pub source: &'static str,
}

pub const BUILTINS: &[Builtin] = &[
    Builtin { key: "aurora", title: "Aurora", source: include_str!("../scenes/aurora.glsl") },
    Builtin { key: "pulse", title: "Pulse", source: include_str!("../scenes/pulse.glsl") },
    Builtin { key: "nebula", title: "Nebula", source: include_str!("../scenes/nebula.glsl") },
];

pub fn builtin_source(key: &str) -> Option<&'static str> {
    BUILTINS.iter().find(|b| b.key == key).map(|b| b.source)
}

const FALLBACK: &str = "void mainImage(out vec4 c, in vec2 p) {
    vec2 uv = p / iResolution.xy;
    c = vec4(mix(vec3(0.05, 0.05, 0.12), vec3(0.20, 0.08, 0.25), uv.y), 1.0);
}";

type GetProcAddress = unsafe extern "C" fn(*const std::ffi::c_char) -> *const std::ffi::c_void;

struct GlLoader {
    egl_get_proc_address: Option<GetProcAddress>,
    libraries: Vec<libloading::Library>,
}

static GL_LOADER: std::sync::OnceLock<GlLoader> = std::sync::OnceLock::new();

/// Opens the system GL libraries once. GTK draws through EGL on Wayland, so
/// entry points come from `eglGetProcAddress`, with `dlsym` as a fallback.
pub fn load_gl() {
    GL_LOADER.get_or_init(|| {
        let open = |name: &str| unsafe { libloading::Library::new(name) }.ok();
        let egl = open("libEGL.so.1");
        let egl_get_proc_address = egl.as_ref().and_then(|lib| unsafe {
            lib.get::<GetProcAddress>(b"eglGetProcAddress\0").ok().map(|s| *s)
        });
        let libraries = [egl, open("libGLESv2.so.2"), open("libGL.so.1")]
            .into_iter()
            .flatten()
            .collect();
        GlLoader { egl_get_proc_address, libraries }
    });
}

fn proc_address(name: &str) -> *const std::ffi::c_void {
    let Some(loader) = GL_LOADER.get() else { return std::ptr::null() };
    let Ok(cname) = std::ffi::CString::new(name) else { return std::ptr::null() };
    if let Some(get) = loader.egl_get_proc_address {
        let ptr = unsafe { get(cname.as_ptr()) };
        if !ptr.is_null() {
            return ptr;
        }
    }
    for lib in &loader.libraries {
        if let Ok(sym) = unsafe { lib.get::<*const std::ffi::c_void>(cname.as_bytes_with_nul()) } {
            return *sym;
        }
    }
    std::ptr::null()
}

/// A pausable clock shared by every display showing the same scene.
#[derive(Default)]
pub struct Clock {
    base: Cell<f64>,
    started: Cell<Option<Instant>>,
}

impl Clock {
    pub fn play(&self) {
        if self.started.get().is_none() {
            self.started.set(Some(Instant::now()));
        }
    }
    pub fn pause(&self) {
        if let Some(start) = self.started.take() {
            self.base.set(self.base.get() + start.elapsed().as_secs_f64());
        }
    }
    pub fn running(&self) -> bool {
        self.started.get().is_some()
    }
    pub fn time(&self) -> f64 {
        self.base.get() + self.started.get().map_or(0.0, |s| s.elapsed().as_secs_f64())
    }
}

pub struct Scene {
    pub source: String,
    pub uses_audio: bool,
    pub clock: Clock,
}

impl Scene {
    pub fn new(source: String) -> Rc<Self> {
        let uses_audio = ["iSpectrum", "iLevel", "iBass"].iter().any(|u| source.contains(u));
        Rc::new(Self { source, uses_audio, clock: Clock::default() })
    }
}

struct Program {
    gl: Rc<glow::Context>,
    program: glow::Program,
    vao: glow::VertexArray,
    u_time: Option<glow::UniformLocation>,
    u_delta: Option<glow::UniformLocation>,
    u_frame: Option<glow::UniformLocation>,
    u_res: Option<glow::UniformLocation>,
    u_mouse: Option<glow::UniformLocation>,
    u_spectrum: Option<glow::UniformLocation>,
    u_level: Option<glow::UniformLocation>,
    u_bass: Option<glow::UniformLocation>,
}

fn header(es: bool) -> &'static str {
    if es {
        "#version 300 es\nprecision highp float;\nprecision highp int;\n"
    } else {
        "#version 330 core\n"
    }
}

fn fragment_source(user: &str, es: bool) -> String {
    format!(
        "{}uniform float iTime;\nuniform float iTimeDelta;\nuniform int iFrame;\n\
         uniform vec3 iResolution;\nuniform vec4 iMouse;\nuniform float iSpectrum[{BANDS}];\n\
         uniform float iLevel;\nuniform float iBass;\nout vec4 frontrise_FragColor;\n#line 1\n{}\n\
         void main() {{ vec4 c = vec4(0.0); mainImage(c, gl_FragCoord.xy); frontrise_FragColor = vec4(c.rgb, 1.0); }}\n",
        header(es),
        user
    )
}

fn vertex_source(es: bool) -> String {
    format!(
        "{}void main() {{ vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));\n\
         gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0); }}\n",
        header(es)
    )
}

unsafe fn compile(gl: &glow::Context, kind: u32, src: &str) -> Result<glow::Shader, String> {
    let shader = gl.create_shader(kind)?;
    gl.shader_source(shader, src);
    gl.compile_shader(shader);
    if !gl.get_shader_compile_status(shader) {
        let log = gl.get_shader_info_log(shader);
        gl.delete_shader(shader);
        return Err(log);
    }
    Ok(shader)
}

unsafe fn link(gl: Rc<glow::Context>, user: &str, es: bool) -> Result<Program, String> {
    let vs = compile(&gl, glow::VERTEX_SHADER, &vertex_source(es))?;
    let fs = match compile(&gl, glow::FRAGMENT_SHADER, &fragment_source(user, es)) {
        Ok(fs) => fs,
        Err(e) => {
            gl.delete_shader(vs);
            return Err(e);
        }
    };
    let program = gl.create_program()?;
    gl.attach_shader(program, vs);
    gl.attach_shader(program, fs);
    gl.link_program(program);
    gl.delete_shader(vs);
    gl.delete_shader(fs);
    if !gl.get_program_link_status(program) {
        let log = gl.get_program_info_log(program);
        gl.delete_program(program);
        return Err(log);
    }
    let vao = gl.create_vertex_array()?;
    let loc = |name: &str| gl.get_uniform_location(program, name);
    Ok(Program {
        u_time: loc("iTime"),
        u_delta: loc("iTimeDelta"),
        u_frame: loc("iFrame"),
        u_res: loc("iResolution"),
        u_mouse: loc("iMouse"),
        u_spectrum: loc("iSpectrum"),
        u_level: loc("iLevel"),
        u_bass: loc("iBass"),
        program,
        vao,
        gl,
    })
}

/// A widget that renders `scene` continuously while its clock runs.
pub fn widget(scene: Rc<Scene>, audio: Rc<Audio>) -> gtk::GLArea {
    let area = gtk::GLArea::new();
    area.set_has_depth_buffer(false);
    area.set_has_stencil_buffer(false);
    area.set_hexpand(true);
    area.set_vexpand(true);

    let program: Rc<RefCell<Option<Program>>> = Rc::default();
    let frame = Rc::new(Cell::new(0i32));
    let last_time = Rc::new(Cell::new(0f64));

    area.connect_realize(glib::clone!(
        #[strong]
        scene,
        #[strong]
        program,
        move |area| {
            area.make_current();
            if let Some(e) = area.error() {
                eprintln!("frontrise: OpenGL unavailable for scenes: {e}");
                return;
            }
            let es = area.context().is_some_and(|c| c.uses_es());
            let gl = Rc::new(unsafe {
                glow::Context::from_loader_function(proc_address)
            });
            let built = unsafe { link(gl.clone(), &scene.source, es) }.or_else(|e| {
                eprintln!("frontrise: scene failed to compile, showing a fallback:\n{e}");
                unsafe { link(gl, FALLBACK, es) }
            });
            match built {
                Ok(p) => *program.borrow_mut() = Some(p),
                Err(e) => eprintln!("frontrise: fallback scene failed too: {e}"),
            }
        }
    ));

    area.connect_unrealize(glib::clone!(
        #[strong]
        program,
        move |area| {
            area.make_current();
            if let Some(p) = program.borrow_mut().take() {
                unsafe {
                    p.gl.delete_program(p.program);
                    p.gl.delete_vertex_array(p.vao);
                }
            }
        }
    ));

    area.connect_render(glib::clone!(
        #[strong]
        scene,
        #[strong]
        program,
        #[strong]
        audio,
        move |area, _ctx| {
            let scale = area.scale_factor() as f32;
            let (w, h) = (area.width() as f32 * scale, area.height() as f32 * scale);
            unsafe {
                if let Some(p) = program.borrow().as_ref() {
                    let gl = &p.gl;
                    let t = scene.clock.time();
                    gl.clear_color(0.0, 0.0, 0.0, 1.0);
                    gl.clear(glow::COLOR_BUFFER_BIT);
                    gl.use_program(Some(p.program));
                    gl.uniform_1_f32(p.u_time.as_ref(), t as f32);
                    gl.uniform_1_f32(p.u_delta.as_ref(), (t - last_time.get()).max(0.0) as f32);
                    gl.uniform_1_i32(p.u_frame.as_ref(), frame.get());
                    gl.uniform_3_f32(p.u_res.as_ref(), w, h, 1.0);
                    gl.uniform_4_f32(p.u_mouse.as_ref(), 0.0, 0.0, 0.0, 0.0);
                    gl.uniform_1_f32_slice(p.u_spectrum.as_ref(), &audio.bands.borrow()[..]);
                    gl.uniform_1_f32(p.u_level.as_ref(), audio.level.get());
                    gl.uniform_1_f32(p.u_bass.as_ref(), audio.bass.get());
                    gl.bind_vertex_array(Some(p.vao));
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                    gl.bind_vertex_array(None);
                    gl.use_program(None);
                    last_time.set(t);
                    frame.set(frame.get().wrapping_add(1));
                }
            }
            glib::Propagation::Stop
        }
    ));

    // Redraw every frame while playing; when paused the last frame stays.
    area.add_tick_callback(move |area, _clock| {
        if scene.clock.running() {
            area.queue_render();
        }
        glib::ControlFlow::Continue
    });

    area
}

/// Draws one frame of `source` at `seconds` off-screen and returns it as
/// top-down RGBA bytes (`width * height * 4`). Uses a throwaway GL context,
/// released before returning: nothing stays alive afterwards. Main thread only.
pub fn render_still(source: &str, seconds: f64, width: i32, height: i32) -> Result<Vec<u8>, String> {
    let display = gtk::gdk::Display::default().ok_or("no display")?;
    let ctx = display.create_gl_context().map_err(|e| e.to_string())?;
    ctx.realize().map_err(|e| e.to_string())?;
    ctx.make_current();
    let es = ctx.uses_es();
    let gl = Rc::new(unsafe { glow::Context::from_loader_function(proc_address) });
    let result = unsafe { draw_still(&gl, source, es, seconds, width, height) };
    gtk::gdk::GLContext::clear_current();
    result
}

unsafe fn draw_still(
    gl: &Rc<glow::Context>,
    source: &str,
    es: bool,
    seconds: f64,
    width: i32,
    height: i32,
) -> Result<Vec<u8>, String> {
    let p = link(gl.clone(), source, es).or_else(|_| link(gl.clone(), FALLBACK, es))?;
    let texture = gl.create_texture()?;
    gl.bind_texture(glow::TEXTURE_2D, Some(texture));
    gl.tex_image_2d(
        glow::TEXTURE_2D,
        0,
        glow::RGBA8 as i32,
        width,
        height,
        0,
        glow::RGBA,
        glow::UNSIGNED_BYTE,
        glow::PixelUnpackData::Slice(None),
    );
    let fbo = gl.create_framebuffer()?;
    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
    gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(texture), 0);

    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let complete = gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
    if complete {
        gl.viewport(0, 0, width, height);
        gl.clear_color(0.0, 0.0, 0.0, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
        gl.use_program(Some(p.program));
        gl.uniform_1_f32(p.u_time.as_ref(), seconds as f32);
        gl.uniform_1_f32(p.u_delta.as_ref(), 0.0);
        gl.uniform_1_i32(p.u_frame.as_ref(), 0);
        gl.uniform_3_f32(p.u_res.as_ref(), width as f32, height as f32, 1.0);
        gl.uniform_4_f32(p.u_mouse.as_ref(), 0.0, 0.0, 0.0, 0.0);
        gl.uniform_1_f32_slice(p.u_spectrum.as_ref(), &[0.0; BANDS]);
        gl.uniform_1_f32(p.u_level.as_ref(), 0.0);
        gl.uniform_1_f32(p.u_bass.as_ref(), 0.0);
        gl.bind_vertex_array(Some(p.vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.use_program(None);
        gl.read_pixels(0, 0, width, height, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut pixels)));
    }

    gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    gl.delete_framebuffer(fbo);
    gl.delete_texture(texture);
    gl.delete_program(p.program);
    gl.delete_vertex_array(p.vao);
    if !complete {
        return Err("off-screen framebuffer unsupported".into());
    }

    // GL rows run bottom-up; images are top-down.
    let stride = (width * 4) as usize;
    let mut flipped = Vec::with_capacity(pixels.len());
    for row in pixels.chunks_exact(stride).rev() {
        flipped.extend_from_slice(row);
    }
    Ok(flipped)
}
