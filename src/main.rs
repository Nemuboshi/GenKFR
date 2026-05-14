use anyhow::{anyhow, bail, Context, Result};
use libloading::{Library, Symbol};
use std::env;
use std::ffi::{CStr, CString};
use std::fs;
use std::io::Write;
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};

const VAPOURSYNTH_API_VERSION: i32 = (3 << 16) | 6;
const PF_YUV420P8: i32 = 3000010;
const PA_REPLACE: i32 = 0;
const SCENE_PROP: &str = "_SceneChangePrev";
const TARGET_HEIGHT: i32 = 360;

#[repr(C)]
struct VSFrameRef {
    _private: [u8; 0],
}
#[repr(C)]
struct VSNodeRef {
    _private: [u8; 0],
}
#[repr(C)]
struct VSCore {
    _private: [u8; 0],
}
#[repr(C)]
struct VSPlugin {
    _private: [u8; 0],
}
#[repr(C)]
struct VSMap {
    _private: [u8; 0],
}
#[repr(C)]
struct VSFuncRef {
    _private: [u8; 0],
}
#[repr(C)]
struct VSFrameContext {
    _private: [u8; 0],
}
#[repr(C)]
struct VSNode {
    _private: [u8; 0],
}

#[repr(C)]
struct VSFormat {
    name: [c_char; 32],
    id: c_int,
    color_family: c_int,
    sample_type: c_int,
    bits_per_sample: c_int,
    bytes_per_sample: c_int,
    sub_sampling_w: c_int,
    sub_sampling_h: c_int,
    num_planes: c_int,
}

#[repr(C)]
struct VSVideoInfo {
    format: *const VSFormat,
    fps_num: i64,
    fps_den: i64,
    width: c_int,
    height: c_int,
    num_frames: c_int,
    flags: c_int,
}

#[repr(C)]
struct VSCoreInfo {
    version_string: *const c_char,
    core: c_int,
    api: c_int,
    num_threads: c_int,
    max_framebuffer_size: i64,
    used_framebuffer_size: i64,
}

type VSMessageHandler = unsafe extern "system" fn(c_int, *const c_char, *mut c_void);
type VSMessageHandlerFree = unsafe extern "system" fn(*mut c_void);
type VSFrameDoneCallback = unsafe extern "system" fn(
    *mut c_void,
    *const VSFrameRef,
    c_int,
    *mut VSNodeRef,
    *const c_char,
);

type VSGetVapourSynthAPI = unsafe extern "system" fn(c_int) -> *const VSAPI;

#[repr(C)]
struct VSAPI {
    create_core: unsafe extern "system" fn(c_int) -> *mut VSCore,
    free_core: unsafe extern "system" fn(*mut VSCore),
    get_core_info: unsafe extern "system" fn(*mut VSCore) -> *const VSCoreInfo,

    clone_frame_ref: unsafe extern "system" fn(*const VSFrameRef) -> *const VSFrameRef,
    clone_node_ref: unsafe extern "system" fn(*mut VSNodeRef) -> *mut VSNodeRef,
    clone_func_ref: unsafe extern "system" fn(*mut VSFuncRef) -> *mut VSFuncRef,

    free_frame: unsafe extern "system" fn(*const VSFrameRef),
    free_node: unsafe extern "system" fn(*mut VSNodeRef),
    free_func: unsafe extern "system" fn(*mut VSFuncRef),

    new_video_frame:
        unsafe extern "system" fn(*const VSFormat, c_int, c_int, *const VSFrameRef, *mut VSCore) -> *mut VSFrameRef,
    copy_frame: unsafe extern "system" fn(*const VSFrameRef, *mut VSCore) -> *mut VSFrameRef,
    copy_frame_props: unsafe extern "system" fn(*const VSFrameRef, *mut VSFrameRef, *mut VSCore),

    register_function: unsafe extern "system" fn(
        *const c_char,
        *const c_char,
        unsafe extern "system" fn(*const VSMap, *mut VSMap, *mut c_void, *mut VSCore, *const VSAPI),
        *mut c_void,
        *mut VSPlugin,
    ),
    get_plugin_by_id: unsafe extern "system" fn(*const c_char, *mut VSCore) -> *mut VSPlugin,
    get_plugin_by_ns: unsafe extern "system" fn(*const c_char, *mut VSCore) -> *mut VSPlugin,
    get_plugins: unsafe extern "system" fn(*mut VSCore) -> *mut VSMap,
    get_functions: unsafe extern "system" fn(*mut VSPlugin) -> *mut VSMap,
    create_filter: unsafe extern "system" fn(
        *const VSMap,
        *mut VSMap,
        *const c_char,
        unsafe extern "system" fn(*mut VSMap, *mut VSMap, *mut *mut c_void, *mut VSNode, *mut VSCore, *const VSAPI),
        unsafe extern "system" fn(c_int, c_int, *mut *mut c_void, *mut *mut c_void, *mut VSFrameContext, *mut VSCore, *const VSAPI) -> *const VSFrameRef,
        unsafe extern "system" fn(*mut c_void, *mut VSCore, *const VSAPI),
        c_int,
        c_int,
        *mut c_void,
        *mut VSCore,
    ),
    set_error: unsafe extern "system" fn(*mut VSMap, *const c_char),
    get_error: unsafe extern "system" fn(*const VSMap) -> *const c_char,
    set_filter_error: unsafe extern "system" fn(*const c_char, *mut VSFrameContext),
    invoke: unsafe extern "system" fn(*mut VSPlugin, *const c_char, *const VSMap) -> *mut VSMap,

    get_format_preset: unsafe extern "system" fn(c_int, *mut VSCore) -> *const VSFormat,
    register_format: unsafe extern "system" fn(c_int, c_int, c_int, c_int, c_int, *mut VSCore) -> *const VSFormat,

    get_frame: unsafe extern "system" fn(c_int, *mut VSNodeRef, *mut c_char, c_int) -> *const VSFrameRef,
    get_frame_async: unsafe extern "system" fn(c_int, *mut VSNodeRef, VSFrameDoneCallback, *mut c_void),
    get_frame_filter: unsafe extern "system" fn(c_int, *mut VSNodeRef, *mut VSFrameContext) -> *const VSFrameRef,
    request_frame_filter: unsafe extern "system" fn(c_int, *mut VSNodeRef, *mut VSFrameContext),
    query_completed_frame: unsafe extern "system" fn(*mut *mut VSNodeRef, *mut c_int, *mut VSFrameContext),
    release_frame_early: unsafe extern "system" fn(*mut VSNodeRef, c_int, *mut VSFrameContext),

    get_stride: unsafe extern "system" fn(*const VSFrameRef, c_int) -> c_int,
    get_read_ptr: unsafe extern "system" fn(*const VSFrameRef, c_int) -> *const u8,
    get_write_ptr: unsafe extern "system" fn(*mut VSFrameRef, c_int) -> *mut u8,

    create_func: unsafe extern "system" fn(
        unsafe extern "system" fn(*const VSMap, *mut VSMap, *mut c_void, *mut VSCore, *const VSAPI),
        *mut c_void,
        unsafe extern "system" fn(*mut c_void),
        *mut VSCore,
        *const VSAPI,
    ) -> *mut VSFuncRef,
    call_func: unsafe extern "system" fn(*mut VSFuncRef, *const VSMap, *mut VSMap, *mut VSCore, *const VSAPI),

    create_map: unsafe extern "system" fn() -> *mut VSMap,
    free_map: unsafe extern "system" fn(*mut VSMap),
    clear_map: unsafe extern "system" fn(*mut VSMap),

    get_video_info: unsafe extern "system" fn(*mut VSNodeRef) -> *const VSVideoInfo,
    set_video_info: unsafe extern "system" fn(*const VSVideoInfo, c_int, *mut VSNode),
    get_frame_format: unsafe extern "system" fn(*const VSFrameRef) -> *const VSFormat,
    get_frame_width: unsafe extern "system" fn(*const VSFrameRef, c_int) -> c_int,
    get_frame_height: unsafe extern "system" fn(*const VSFrameRef, c_int) -> c_int,
    get_frame_props_ro: unsafe extern "system" fn(*const VSFrameRef) -> *const VSMap,
    get_frame_props_rw: unsafe extern "system" fn(*mut VSFrameRef) -> *mut VSMap,

    prop_num_keys: unsafe extern "system" fn(*const VSMap) -> c_int,
    prop_get_key: unsafe extern "system" fn(*const VSMap, c_int) -> *const c_char,
    prop_num_elements: unsafe extern "system" fn(*const VSMap, *const c_char) -> c_int,
    prop_get_type: unsafe extern "system" fn(*const VSMap, *const c_char) -> c_char,

    prop_get_int: unsafe extern "system" fn(*const VSMap, *const c_char, c_int, *mut c_int) -> i64,
    prop_get_float: unsafe extern "system" fn(*const VSMap, *const c_char, c_int, *mut c_int) -> f64,
    prop_get_data: unsafe extern "system" fn(*const VSMap, *const c_char, c_int, *mut c_int) -> *const c_char,
    prop_get_data_size: unsafe extern "system" fn(*const VSMap, *const c_char, c_int, *mut c_int) -> c_int,
    prop_get_node: unsafe extern "system" fn(*const VSMap, *const c_char, c_int, *mut c_int) -> *mut VSNodeRef,
    prop_get_frame: unsafe extern "system" fn(*const VSMap, *const c_char, c_int, *mut c_int) -> *const VSFrameRef,
    prop_get_func: unsafe extern "system" fn(*const VSMap, *const c_char, c_int, *mut c_int) -> *mut VSFuncRef,

    prop_delete_key: unsafe extern "system" fn(*mut VSMap, *const c_char) -> c_int,
    prop_set_int: unsafe extern "system" fn(*mut VSMap, *const c_char, i64, c_int) -> c_int,
    prop_set_float: unsafe extern "system" fn(*mut VSMap, *const c_char, f64, c_int) -> c_int,
    prop_set_data: unsafe extern "system" fn(*mut VSMap, *const c_char, *const c_char, c_int, c_int) -> c_int,
    prop_set_node: unsafe extern "system" fn(*mut VSMap, *const c_char, *mut VSNodeRef, c_int) -> c_int,
    prop_set_frame: unsafe extern "system" fn(*mut VSMap, *const c_char, *const VSFrameRef, c_int) -> c_int,
    prop_set_func: unsafe extern "system" fn(*mut VSMap, *const c_char, *mut VSFuncRef, c_int) -> c_int,

    set_max_cache_size: unsafe extern "system" fn(i64, *mut VSCore) -> i64,
    get_output_index: unsafe extern "system" fn(*mut VSFrameContext) -> c_int,
    new_video_frame2: unsafe extern "system" fn(
        *const VSFormat,
        c_int,
        c_int,
        *const *const VSFrameRef,
        *const c_int,
        *const VSFrameRef,
        *mut VSCore,
    ) -> *mut VSFrameRef,
    set_message_handler: unsafe extern "system" fn(VSMessageHandler, *mut c_void),
    set_thread_count: unsafe extern "system" fn(c_int, *mut VSCore) -> c_int,

    get_plugin_path: unsafe extern "system" fn(*const VSPlugin) -> *const c_char,

    prop_get_int_array: unsafe extern "system" fn(*const VSMap, *const c_char, *mut c_int) -> *const i64,
    prop_get_float_array: unsafe extern "system" fn(*const VSMap, *const c_char, *mut c_int) -> *const f64,

    prop_set_int_array: unsafe extern "system" fn(*mut VSMap, *const c_char, *const i64, c_int) -> c_int,
    prop_set_float_array: unsafe extern "system" fn(*mut VSMap, *const c_char, *const f64, c_int) -> c_int,

    log_message: unsafe extern "system" fn(c_int, *const c_char),

    add_message_handler: unsafe extern "system" fn(VSMessageHandler, VSMessageHandlerFree, *mut c_void) -> c_int,
    remove_message_handler: unsafe extern "system" fn(c_int) -> c_int,
    get_core_info2: unsafe extern "system" fn(*mut VSCore, *mut VSCoreInfo),
}

struct VapourSynth {
    _lib: Library,
    api: &'static VSAPI,
}

struct Core<'a> {
    api: &'a VSAPI,
    raw: *mut VSCore,
}

struct OwnedMap<'a> {
    api: &'a VSAPI,
    raw: *mut VSMap,
}

struct OwnedNode<'a> {
    api: &'a VSAPI,
    raw: *mut VSNodeRef,
}

impl VapourSynth {
    fn load(path: &Path) -> Result<Self> {
        unsafe {
            let lib = Library::new(path).with_context(|| format!("failed to load {}", path.display()))?;
            let get_api: Symbol<VSGetVapourSynthAPI> = lib
                .get(b"getVapourSynthAPI\0")
                .context("failed to load getVapourSynthAPI symbol")?;
            let api = get_api(VAPOURSYNTH_API_VERSION);
            if api.is_null() {
                bail!("getVapourSynthAPI returned null for API version {VAPOURSYNTH_API_VERSION}");
            }
            Ok(Self { _lib: lib, api: &*api })
        }
    }
}

impl<'a> Core<'a> {
    fn new(api: &'a VSAPI) -> Result<Self> {
        unsafe {
            let raw = (api.create_core)(0);
            if raw.is_null() {
                bail!("createCore returned null");
            }
            Ok(Self { api, raw })
        }
    }

    fn get_plugin_by_ns(&self, namespace: &str) -> Result<*mut VSPlugin> {
        let ns = CString::new(namespace)?;
        let plugin = unsafe { (self.api.get_plugin_by_ns)(ns.as_ptr(), self.raw) };
        if plugin.is_null() {
            bail!("plugin namespace not found: {namespace}");
        }
        Ok(plugin)
    }

    fn has_plugin(&self, namespace: &str) -> Result<bool> {
        let ns = CString::new(namespace)?;
        let plugin = unsafe { (self.api.get_plugin_by_ns)(ns.as_ptr(), self.raw) };
        Ok(!plugin.is_null())
    }

    fn invoke_node(&self, plugin_ns: &str, function: &str, args: &OwnedMap<'a>) -> Result<OwnedNode<'a>> {
        let plugin = self.get_plugin_by_ns(plugin_ns)?;
        let function_c = CString::new(function)?;
        let result = unsafe { (self.api.invoke)(plugin, function_c.as_ptr(), args.raw) };
        if result.is_null() {
            bail!("invoke returned null for {plugin_ns}.{function}");
        }
        let result_map = OwnedMap { api: self.api, raw: result };
        result_map.check_error(&format!("{plugin_ns}.{function}"))?;
        result_map.get_node("clip")
    }

    fn load_plugin(&self, namespace: &str, plugin_path: &Path) -> Result<()> {
        if self.has_plugin(namespace)? {
            return Ok(());
        }

        let std = self.get_plugin_by_ns("std")?;
        let args = OwnedMap::new(self.api);
        args.set_data("path", &path_cstring(plugin_path)?)?;
        let func = CString::new("LoadPlugin")?;
        let result = unsafe { (self.api.invoke)(std, func.as_ptr(), args.raw) };
        if result.is_null() {
            bail!("std.LoadPlugin returned null for {}", plugin_path.display());
        }
        let result_map = OwnedMap { api: self.api, raw: result };
        result_map.check_error(&format!("std.LoadPlugin({})", plugin_path.display()))?;

        if !self.has_plugin(namespace)? {
            bail!("plugin {namespace} still unavailable after loading {}", plugin_path.display());
        }
        Ok(())
    }

    fn source(&self, input: &Path) -> Result<OwnedNode<'a>> {
        let args = OwnedMap::new(self.api);
        args.set_data("source", &path_cstring(input)?)?;
        self.invoke_node("ffms2", "Source", &args)
    }

    fn resize_to_scxvid(&self, node: &OwnedNode<'a>, width: i32, height: i32) -> Result<OwnedNode<'a>> {
        let args = OwnedMap::new(self.api);
        args.set_node("clip", node.raw)?;
        args.set_int("width", width as i64)?;
        args.set_int("height", height as i64)?;
        args.set_int("format", PF_YUV420P8 as i64)?;
        self.invoke_node("resize", "Bilinear", &args)
    }

    fn scxvid(&self, node: &OwnedNode<'a>, prop_name: &str) -> Result<OwnedNode<'a>> {
        let args = OwnedMap::new(self.api);
        args.set_node("clip", node.raw)?;
        args.set_data("prop", &CString::new(prop_name)?)?;
        self.invoke_node("scxvid", "Scxvid", &args)
    }
}

impl<'a> Drop for Core<'a> {
    fn drop(&mut self) {
        unsafe {
            (self.api.free_core)(self.raw);
        }
    }
}

impl<'a> OwnedMap<'a> {
    fn new(api: &'a VSAPI) -> Self {
        let raw = unsafe { (api.create_map)() };
        Self { api, raw }
    }

    fn check_error(&self, context: &str) -> Result<()> {
        unsafe {
            let err = (self.api.get_error)(self.raw);
            if err.is_null() {
                Ok(())
            } else {
                Err(anyhow!("{context}: {}", CStr::from_ptr(err).to_string_lossy()))
            }
        }
    }

    fn set_int(&self, key: &str, value: i64) -> Result<()> {
        let key = CString::new(key)?;
        let rc = unsafe { (self.api.prop_set_int)(self.raw, key.as_ptr(), value, PA_REPLACE) };
        if rc != 0 {
            bail!("propSetInt failed for key {key:?} with code {rc}");
        }
        Ok(())
    }

    fn set_data(&self, key: &str, value: &CString) -> Result<()> {
        let key = CString::new(key)?;
        let bytes = value.as_bytes_with_nul();
        let rc = unsafe {
            (self.api.prop_set_data)(
                self.raw,
                key.as_ptr(),
                bytes.as_ptr().cast(),
                bytes.len() as c_int,
                PA_REPLACE,
            )
        };
        if rc != 0 {
            bail!("propSetData failed for key {key:?} with code {rc}");
        }
        Ok(())
    }

    fn set_node(&self, key: &str, node: *mut VSNodeRef) -> Result<()> {
        let key = CString::new(key)?;
        let rc = unsafe { (self.api.prop_set_node)(self.raw, key.as_ptr(), node, PA_REPLACE) };
        if rc != 0 {
            bail!("propSetNode failed for key {key:?} with code {rc}");
        }
        Ok(())
    }

    fn get_node(&self, key: &str) -> Result<OwnedNode<'a>> {
        let key = CString::new(key)?;
        let mut err = 0;
        let raw = unsafe { (self.api.prop_get_node)(self.raw, key.as_ptr(), 0, &mut err) };
        if err != 0 || raw.is_null() {
            bail!("propGetNode failed for key {:?} with error {}", key, err);
        }
        Ok(OwnedNode { api: self.api, raw })
    }
}

impl<'a> Drop for OwnedMap<'a> {
    fn drop(&mut self) {
        unsafe {
            (self.api.free_map)(self.raw);
        }
    }
}

impl<'a> Drop for OwnedNode<'a> {
    fn drop(&mut self) {
        unsafe {
            (self.api.free_node)(self.raw);
        }
    }
}

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = args
        .next()
        .map(PathBuf::from)
        .context("usage: genkfr <input-video> [output-kf.txt]")?;
    let output = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("{}.kf.txt", input.display())));

    let runtime = RuntimeLayout::discover()?;
    let vs = VapourSynth::load(&runtime.vapoursynth_dll)?;
    let core = Core::new(vs.api)?;

    core.load_plugin("ffms2", &runtime.ffms2_dll)?;
    core.load_plugin("scxvid", &runtime.scxvid_dll)?;

    let input = canonicalize_lossy(&input)?;
    let source = core.source(&input)?;
    let info = video_info(vs.api, source.raw)?;
    let target_width = align_width_4(info.width, info.height, TARGET_HEIGHT);
    let resized = core.resize_to_scxvid(&source, target_width, TARGET_HEIGHT)?;
    let analyzed = core.scxvid(&resized, SCENE_PROP)?;

    let keyframes = collect_keyframes(vs.api, analyzed.raw, info.num_frames)?;
    write_kf_file(&output, info.fps(), &keyframes)?;

    println!("input: {}", input.display());
    println!("output: {}", output.display());
    println!("runtime: {}", runtime.root.display());
    println!("frames: {}", info.num_frames);
    println!("fps: {:.6}", info.fps());
    println!("keyframes: {}", keyframes.len());

    Ok(())
}

fn collect_keyframes(api: &VSAPI, node: *mut VSNodeRef, num_frames: i32) -> Result<Vec<i32>> {
    let mut keyframes = Vec::new();
    let prop = CString::new(SCENE_PROP)?;
    let mut error_buf = vec![0 as c_char; 4096];

    for n in 0..num_frames {
        if n % 1000 == 0 {
            eprintln!("processing frame {n}/{num_frames}");
        }
        let frame = unsafe { (api.get_frame)(n, node, error_buf.as_mut_ptr(), error_buf.len() as c_int) };
        if frame.is_null() {
            let msg = unsafe { CStr::from_ptr(error_buf.as_ptr()) }.to_string_lossy().into_owned();
            bail!("getFrame({n}) failed: {msg}");
        }

        let props = unsafe { (api.get_frame_props_ro)(frame) };
        let mut prop_err = 0;
        let flag = unsafe { (api.prop_get_int)(props, prop.as_ptr(), 0, &mut prop_err) };
        unsafe { (api.free_frame)(frame) };

        if prop_err == 0 && flag != 0 {
            keyframes.push(n);
        }
    }

    Ok(keyframes)
}

fn write_kf_file(path: &Path, fps: f64, keyframes: &[i32]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut file = fs::File::create(path).with_context(|| format!("failed to create {}", path.display()))?;
    writeln!(file, "# keyframe format v1")?;
    writeln!(file, "fps {:.6}", fps)?;
    for frame in keyframes {
        writeln!(file, "{frame}")?;
    }
    Ok(())
}

fn video_info(api: &VSAPI, node: *mut VSNodeRef) -> Result<VideoInfo> {
    let vi = unsafe { (api.get_video_info)(node) };
    if vi.is_null() {
        bail!("getVideoInfo returned null");
    }
    let vi = unsafe { &*vi };
    if vi.width <= 0 || vi.height <= 0 || vi.num_frames <= 0 {
        bail!(
            "invalid video info: width={}, height={}, num_frames={}",
            vi.width,
            vi.height,
            vi.num_frames
        );
    }
    if vi.fps_num <= 0 || vi.fps_den <= 0 {
        bail!("invalid fps: {}/{}", vi.fps_num, vi.fps_den);
    }
    Ok(VideoInfo {
        width: vi.width,
        height: vi.height,
        num_frames: vi.num_frames,
        fps_num: vi.fps_num,
        fps_den: vi.fps_den,
    })
}

struct VideoInfo {
    width: i32,
    height: i32,
    num_frames: i32,
    fps_num: i64,
    fps_den: i64,
}

impl VideoInfo {
    fn fps(&self) -> f64 {
        self.fps_num as f64 / self.fps_den as f64
    }
}

fn align_width_4(src_w: i32, src_h: i32, target_h: i32) -> i32 {
    let scaled = (src_w as f64 * target_h as f64) / src_h as f64;
    let rounded = (scaled / 4.0).round() as i32 * 4;
    rounded.max(16)
}

fn path_cstring(path: &Path) -> Result<CString> {
    CString::new(path.to_string_lossy().to_string()).context("path contains interior NUL")
}

fn canonicalize_lossy(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).with_context(|| format!("failed to resolve {}", path.display()))
}

struct RuntimeLayout {
    root: PathBuf,
    vapoursynth_dll: PathBuf,
    ffms2_dll: PathBuf,
    scxvid_dll: PathBuf,
}

impl RuntimeLayout {
    fn discover() -> Result<Self> {
        let exe = env::current_exe().context("failed to get current executable path")?;
        let exe_dir = exe.parent().context("executable has no parent directory")?;
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        let candidates = [
            exe_dir.join("runtime"),
            exe_dir.join(r"..\runtime"),
            exe_dir.join(r"..\..\runtime"),
            manifest_dir.join("runtime"),
        ];

        for candidate in candidates {
            let root = normalize_candidate(&candidate);
            let vapoursynth_dll = root.join("libvapoursynth.dll");
            let ffms2_dll = root.join("plugins").join("ffms2.dll");
            let scxvid_dll = root.join("plugins").join("scxvid.dll");
            if vapoursynth_dll.is_file() && ffms2_dll.is_file() && scxvid_dll.is_file() {
                return Ok(Self {
                    root,
                    vapoursynth_dll,
                    ffms2_dll,
                    scxvid_dll,
                });
            }
        }

        bail!(
            "private runtime not found; expected runtime/libvapoursynth.dll and runtime/plugins/{{ffms2.dll,scxvid.dll}} near the executable or crate root"
        )
    }
}

fn normalize_candidate(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
