extern crate bindgen;
extern crate cc;
extern crate pkg_config;

use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::Command;
use std::str;

use bindgen::callbacks::{IntKind, MacroParsingBehavior, ParseCallbacks};

#[derive(Debug)]
struct Library {
    name: &'static str,
    is_feature: bool,
}

impl Library {
    fn feature_name(&self) -> Option<String> {
        if self.is_feature {
            Some("CARGO_FEATURE_".to_string() + &self.name.to_uppercase())
        } else {
            None
        }
    }
}

static LIBRARIES: &[Library] = &[
    Library {
        name: "avcodec",
        is_feature: true,
    },
    Library {
        name: "avdevice",
        is_feature: true,
    },
    Library {
        name: "avfilter",
        is_feature: true,
    },
    Library {
        name: "avformat",
        is_feature: true,
    },
    Library {
        name: "avresample",
        is_feature: true,
    },
    Library {
        name: "avutil",
        is_feature: false,
    },
    Library {
        name: "postproc",
        is_feature: true,
    },
    Library {
        name: "swresample",
        is_feature: true,
    },
    Library {
        name: "swscale",
        is_feature: true,
    },
];

#[derive(Debug)]
struct Callbacks;

impl ParseCallbacks for Callbacks {
    fn int_macro(&self, name: &str, value: i64) -> Option<IntKind> {
        if value >= i64::min_value() as i64 && value <= i64::max_value() as i64 && name.starts_with("AV_CH") {
            Some(IntKind::ULongLong)
        } else if value >= i32::min_value() as i64
            && value <= i32::max_value() as i64
            && (name.starts_with("AV_CODEC_CAP") || name.starts_with("AV_CODEC_FLAG"))
        {
            Some(IntKind::UInt)
        } else if name.starts_with("AV_ERROR_MAX_STRING_SIZE") {
            Some(IntKind::Custom {
                name: "usize",
                is_signed: false,
            })
        } else if value >= i32::min_value() as i64 && value <= i32::max_value() as i64 {
            Some(IntKind::Int)
        } else {
            None
        }
    }

    // https://github.com/rust-lang/rust-bindgen/issues/687#issuecomment-388277405
    fn will_parse_macro(&self, name: &str) -> MacroParsingBehavior {
        use MacroParsingBehavior::*;

        match name {
            "FP_INFINITE" => Ignore,
            "FP_NAN" => Ignore,
            "FP_NORMAL" => Ignore,
            "FP_SUBNORMAL" => Ignore,
            "FP_ZERO" => Ignore,
            _ => Default,
        }
    }
}

fn version() -> String {
    let major: u8 = env::var("CARGO_PKG_VERSION_MAJOR").unwrap().parse().unwrap();
    let minor: u8 = env::var("CARGO_PKG_VERSION_MINOR").unwrap().parse().unwrap();

    format!("{}.{}", major, minor)
}

fn output() -> PathBuf {
    PathBuf::from(env::var("OUT_DIR").unwrap())
}

fn source() -> PathBuf {
    output().join(format!("ffmpeg-{}", version()))
}

fn search() -> PathBuf {
    let mut absolute = env::current_dir().unwrap();
    absolute.push(&output());
    absolute.push("dist");

    absolute
}

fn switch(configure: &mut Command, feature: &str, name: &str) {
    let arg = if env::var("CARGO_FEATURE_".to_string() + feature).is_ok() {
        "--enable-"
    } else {
        "--disable-"
    };
    configure.arg(arg.to_string() + name);
}

fn build() -> io::Result<()> {
    let mut configure = Command::new(fs::canonicalize("./ffmpeg/configure").unwrap());

    configure.current_dir(&source());
    configure.arg(format!("--prefix={}", search().to_string_lossy()));

    if env::var("TARGET").unwrap() != env::var("HOST").unwrap() {
        configure.arg(format!("--cross-prefix={}-", env::var("TARGET").unwrap()));
    }

    // control debug build
    if env::var("DEBUG").is_ok() {
        configure.arg("--enable-debug");
        configure.arg("--disable-stripping");
    } else {
        configure.arg("--disable-debug");
        configure.arg("--enable-stripping");
    }

    // make it static
    configure.arg("--enable-static");
    configure.arg("--disable-shared");

    configure.arg("--enable-pic");

    // do not build programs since we don't need them
    configure.arg("--disable-programs");

    // yasm is not available on docs.rs. Build crooked version
    if env::var("CARGO_FEATURE_BUILD_DISABLE_X86ASM").is_ok() {
        configure.arg("--disable-x86asm");
    }

    macro_rules! enable {
        ($conf:expr, $feat:expr, $name:expr) => {
            if env::var(concat!("CARGO_FEATURE_", $feat)).is_ok() {
                $conf.arg(concat!("--enable-", $name));
            } else {
                $conf.arg(concat!("--disable-", $name));
            }
        };
    }

    macro_rules! enable_component {
        ($conf:expr, $comp:expr, $name:expr) => {
            $conf.arg(concat!("--enable-", $comp, "=", $name));
        };
    }

    // the binary using ffmpeg-sys must comply with GPL
    switch(&mut configure, "BUILD_LICENSE_GPL", "gpl");

    // the binary using ffmpeg-sys must comply with (L)GPLv3
    switch(&mut configure, "BUILD_LICENSE_VERSION3", "version3");

    // the binary using ffmpeg-sys cannot be redistributed
    switch(&mut configure, "BUILD_LICENSE_NONFREE", "nonfree");

    // configure building libraries based on features
    for lib in LIBRARIES.iter().filter(|lib| lib.is_feature) {
        switch(&mut configure, &lib.name.to_uppercase(), lib.name);
    }

    if env::var(concat!("CARGO_FEATURE_TINY")).is_ok() {
        configure.arg("--disable-everything");
        enable_component!(configure, "protocol", "file");
        enable_component!(configure, "protocol", "pipe");

        if env::var(concat!("CARGO_FEATURE_COMMON_AUDIO")).is_ok() {
            enable_component!(configure, "demuxer", "pcm_f64be");
            enable_component!(configure, "demuxer", "pcm_f64le");
            enable_component!(configure, "demuxer", "pcm_f32be");
            enable_component!(configure, "demuxer", "pcm_f32le");
            enable_component!(configure, "demuxer", "pcm_s32be");
            enable_component!(configure, "demuxer", "pcm_s32le");
            enable_component!(configure, "demuxer", "pcm_s24be");
            enable_component!(configure, "demuxer", "pcm_s24le");
            enable_component!(configure, "demuxer", "pcm_s16be");
            enable_component!(configure, "demuxer", "pcm_s16le");
            enable_component!(configure, "muxer", "pcm_f64be");
            enable_component!(configure, "muxer", "pcm_f64le");
            enable_component!(configure, "muxer", "pcm_f32be");
            enable_component!(configure, "muxer", "pcm_f32le");
            enable_component!(configure, "muxer", "pcm_s32be");
            enable_component!(configure, "muxer", "pcm_s32le");
            enable_component!(configure, "muxer", "pcm_s24be");
            enable_component!(configure, "muxer", "pcm_s24le");
            enable_component!(configure, "muxer", "pcm_s16be");
            enable_component!(configure, "muxer", "pcm_s16le");
            enable_component!(configure, "decoder", "pcm_f32be");
            enable_component!(configure, "decoder", "pcm_f32le");
            enable_component!(configure, "decoder", "pcm_f64be");
            enable_component!(configure, "decoder", "pcm_f64le");
            enable_component!(configure, "decoder", "pcm_s16be");
            enable_component!(configure, "decoder", "pcm_s16be_planar");
            enable_component!(configure, "decoder", "pcm_s16le");
            enable_component!(configure, "decoder", "pcm_s16le_planar");
            enable_component!(configure, "decoder", "pcm_s24be");
            enable_component!(configure, "decoder", "pcm_s24be_planar");
            enable_component!(configure, "decoder", "pcm_s24le");
            enable_component!(configure, "decoder", "pcm_s24le_planar");
            enable_component!(configure, "decoder", "pcm_s32be");
            enable_component!(configure, "decoder", "pcm_s32be_planar");
            enable_component!(configure, "decoder", "pcm_s32le");
            enable_component!(configure, "decoder", "pcm_s32le_planar");
            enable_component!(configure, "encoder", "pcm_f32be");
            enable_component!(configure, "encoder", "pcm_f32le");
            enable_component!(configure, "encoder", "pcm_f64be");
            enable_component!(configure, "encoder", "pcm_f64le");
            enable_component!(configure, "encoder", "pcm_s16be");
            enable_component!(configure, "encoder", "pcm_s16be_planar");
            enable_component!(configure, "encoder", "pcm_s16le");
            enable_component!(configure, "encoder", "pcm_s16le_planar");
            enable_component!(configure, "encoder", "pcm_s24be");
            enable_component!(configure, "encoder", "pcm_s24be_planar");
            enable_component!(configure, "encoder", "pcm_s24le");
            enable_component!(configure, "encoder", "pcm_s24le_planar");
            enable_component!(configure, "encoder", "pcm_s32be");
            enable_component!(configure, "encoder", "pcm_s32be_planar");
            enable_component!(configure, "encoder", "pcm_s32le");
            enable_component!(configure, "encoder", "pcm_s32le_planar");

            enable_component!(configure, "demuxer", "aac");
            enable_component!(configure, "demuxer", "aiff");
            enable_component!(configure, "demuxer", "flac");
            enable_component!(configure, "demuxer", "mov"); // There is no mp4 demuxer
            enable_component!(configure, "demuxer", "mp3");
            enable_component!(configure, "demuxer", "wav");
            enable_component!(configure, "muxer", "aiff");
            enable_component!(configure, "muxer", "flac");
            enable_component!(configure, "muxer", "mp3");
            enable_component!(configure, "muxer", "mov");
            enable_component!(configure, "muxer", "mp4");
            enable_component!(configure, "muxer", "wav");

            enable_component!(configure, "decoder", "aac");
            enable_component!(configure, "decoder", "aac_fixed");
            enable_component!(configure, "decoder", "aac_latm");
            enable_component!(configure, "decoder", "alac");
            enable_component!(configure, "decoder", "flac");
            enable_component!(configure, "decoder", "mp3");
            enable_component!(configure, "encoder", "aac");
            enable_component!(configure, "encoder", "alac");
            enable_component!(configure, "encoder", "flac");

            enable_component!(configure, "parser", "aac");
            enable_component!(configure, "parser", "aac_latm");
            enable_component!(configure, "parser", "flac");
            enable_component!(configure, "parser", "mpegaudio");
            enable_component!(configure, "parser", "vorbis");
        }
    }

    // configure external SSL libraries
    enable!(configure, "BUILD_LIB_GNUTLS", "gnutls");
    enable!(configure, "BUILD_LIB_OPENSSL", "openssl");

    // configure external filters
    enable!(configure, "BUILD_LIB_FONTCONFIG", "fontconfig");
    enable!(configure, "BUILD_LIB_FREI0R", "frei0r");
    enable!(configure, "BUILD_LIB_LADSPA", "ladspa");
    enable!(configure, "BUILD_LIB_ASS", "libass");
    enable!(configure, "BUILD_LIB_FREETYPE", "libfreetype");
    enable!(configure, "BUILD_LIB_FRIBIDI", "libfribidi");
    enable!(configure, "BUILD_LIB_OPENCV", "libopencv");

    // configure external encoders/decoders
    enable!(configure, "BUILD_LIB_CELT", "libcelt");
    enable!(configure, "BUILD_LIB_FDK_AAC", "libfdk-aac");
    enable!(configure, "BUILD_LIB_GSM", "libgsm");
    enable!(configure, "BUILD_LIB_ILBC", "libilbc");
    enable!(configure, "BUILD_LIB_KVAZAAR", "libkvazaar");
    enable!(configure, "BUILD_LIB_MP3LAME", "libmp3lame");
    enable!(configure, "BUILD_LIB_OPENCORE_AMRNB", "libopencore-amrnb");
    enable!(configure, "BUILD_LIB_OPENCORE_AMRWB", "libopencore-amrwb");
    enable!(configure, "BUILD_LIB_OPENH264", "libopenh264");
    enable!(configure, "BUILD_LIB_OPENJPEG", "libopenjpeg");
    enable!(configure, "BUILD_LIB_OPUS", "libopus");
    enable!(configure, "BUILD_LIB_SHINE", "libshine");
    enable!(configure, "BUILD_LIB_SNAPPY", "libsnappy");
    enable!(configure, "BUILD_LIB_SPEEX", "libspeex");
    enable!(configure, "BUILD_LIB_THEORA", "libtheora");
    enable!(configure, "BUILD_LIB_TWOLAME", "libtwolame");
    enable!(configure, "BUILD_LIB_VO_AMRWBENC", "libvo-amrwbenc");
    enable!(configure, "BUILD_LIB_VORBIS", "libvorbis");
    enable!(configure, "BUILD_LIB_VPX", "libvpx");
    enable!(configure, "BUILD_LIB_WAVPACK", "libwavpack");
    enable!(configure, "BUILD_LIB_WEBP", "libwebp");
    enable!(configure, "BUILD_LIB_X264", "libx264");
    enable!(configure, "BUILD_LIB_X265", "libx265");
    enable!(configure, "BUILD_LIB_XVID", "libxvid");

    // other external libraries
    enable!(configure, "BUILD_NVENC", "nvenc");
    enable!(configure, "BUILD_SDL2", "sdl2");

    // configure external protocols
    enable!(configure, "BUILD_LIB_SMBCLIENT", "libsmbclient");
    enable!(configure, "BUILD_LIB_SSH", "libssh");

    // run ./configure
    let output = configure.output().expect(&format!("{:?} failed", configure));
    if !output.status.success() {
        println!("configure: {}", String::from_utf8_lossy(&output.stdout));

        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("configure failed {}", String::from_utf8_lossy(&output.stderr)),
        ));
    }

    // run make
    Command::new("make")
        .arg("-j")
        .arg(env::var("NUM_JOBS").unwrap_or_else(|_| "1".into()))
        .current_dir(&source())
        .status()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "make failed"))?;

    // run make install
    Command::new("make")
        .current_dir(&source())
        .arg("install")
        .status()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "make install failed"))?;

    Ok(())
}

fn check_features(include_paths: Vec<PathBuf>, infos: &Vec<(&'static str, Option<&'static str>, &'static str)>) {
    let mut includes_code = String::new();
    let mut main_code = String::new();

    for &(header, feature, var) in infos {
        if let Some(feature) = feature {
            if env::var(format!("CARGO_FEATURE_{}", feature.to_uppercase())).is_err() {
                continue;
            }
        }

        let include = format!("#include <{}>", header);
        if includes_code.find(&include).is_none() {
            includes_code.push_str(&include);
            includes_code.push_str(&"\n");
        }
        includes_code.push_str(&format!(
            r#"
            #ifndef {var}
            #define {var} 0
            #define {var}_is_defined 0
            #else
            #define {var}_is_defined 1
            #endif
        "#,
            var = var
        ));

        main_code.push_str(&format!(r#"printf("[{var}]%d%d\n", {var}, {var}_is_defined);"#, var = var));
    }

    let version_check_info = [("avutil", 56, 60, 0, 80)];
    for &(lib, begin_version_major, end_version_major, begin_version_minor, end_version_minor) in version_check_info.iter() {
        for version_major in begin_version_major..end_version_major {
            for version_minor in begin_version_minor..end_version_minor {
                main_code.push_str(&format!(
                                r#"printf("[{lib}_version_greater_than_{version_major}_{version_minor}]%d\n", LIB{lib_uppercase}_VERSION_MAJOR > {version_major} || (LIB{lib_uppercase}_VERSION_MAJOR == {version_major} && LIB{lib_uppercase}_VERSION_MINOR > {version_minor}));"#,
                                lib = lib,
                                lib_uppercase = lib.to_uppercase(),
                                version_major = version_major,
                                version_minor = version_minor
                        ));
            }
        }
    }

    let out_dir = output();

    write!(
        File::create(out_dir.join("check.c")).expect("Failed to create file"),
        r#"
            #include <stdio.h>
            {includes_code}

            int main()
            {{
                {main_code}
                return 0;
            }}
           "#,
        includes_code = includes_code,
        main_code = main_code
    )
    .expect("Write failed");

    let executable = out_dir.join(if cfg!(windows) { "check.exe" } else { "check" });
    let mut compiler = cc::Build::new().get_compiler().to_command();

    for dir in include_paths {
        compiler.arg("-I");
        compiler.arg(dir.to_string_lossy().into_owned());
    }
    if !compiler
        .current_dir(&out_dir)
        .arg("-o")
        .arg(&executable)
        .arg("check.c")
        .status()
        .expect("Command failed")
        .success()
    {
        panic!("Compile failed");
    }

    let stdout_raw = Command::new(out_dir.join(&executable))
        .current_dir(&out_dir)
        .output()
        .expect("Check failed")
        .stdout;
    let stdout = str::from_utf8(stdout_raw.as_slice()).unwrap();

    println!("stdout={}", stdout);

    for &(_, feature, var) in infos {
        if let Some(feature) = feature {
            if env::var(format!("CARGO_FEATURE_{}", feature.to_uppercase())).is_err() {
                continue;
            }
        }

        let var_str = format!("[{var}]", var = var);
        let pos = stdout.find(&var_str).expect("Variable not found in output") + var_str.len();
        if &stdout[pos..pos + 1] == "1" {
            println!(r#"cargo:rustc-cfg=feature="{}""#, var.to_lowercase());
            println!(r#"cargo:{}=true"#, var.to_lowercase());
        }

        // Also find out if defined or not (useful for cases where only the definition of a macro
        // can be used as distinction)
        if &stdout[pos + 1..pos + 2] == "1" {
            println!(r#"cargo:rustc-cfg=feature="{}_is_defined""#, var.to_lowercase());
            println!(r#"cargo:{}_is_defined=true"#, var.to_lowercase());
        }
    }

    for &(lib, begin_version_major, end_version_major, begin_version_minor, end_version_minor) in version_check_info.iter() {
        for version_major in begin_version_major..end_version_major {
            for version_minor in begin_version_minor..end_version_minor {
                let search_str = format!(
                    "[{lib}_version_greater_than_{version_major}_{version_minor}]",
                    version_major = version_major,
                    version_minor = version_minor,
                    lib = lib
                );
                let pos = stdout.find(&search_str).expect("Variable not found in output") + search_str.len();

                if &stdout[pos..pos + 1] == "1" {
                    println!(r#"cargo:rustc-cfg=feature="{}""#, &search_str[1..(search_str.len() - 1)]);
                }
            }
        }
    }
}

fn search_include(include_paths: &Vec<PathBuf>, header: &str) -> String {
    for dir in include_paths {
        let include = dir.join(header);
        if fs::metadata(&include).is_ok() {
            return format!("{}", include.as_path().to_str().unwrap());
        }
    }
    format!("/usr/include/{}", header)
}

fn link_to_libraries(statik: bool) {
    let ffmpeg_ty = if statik { "static" } else { "dylib" };

    if cfg!(windows) {
        println!("cargo:rustc-link-lib={}={}", ffmpeg_ty, "libmp3lame-static");
        println!("cargo:rustc-link-lib={}={}", ffmpeg_ty, "libmpghip-static");
    } else {
        println!("cargo:rustc-link-lib={}={}", ffmpeg_ty, "mp3lame");
    }

    for lib in LIBRARIES {
        let feat_is_enabled = lib.feature_name().and_then(|f| env::var(&f).ok()).is_some();
        if !lib.is_feature || feat_is_enabled {
            println!("cargo:rustc-link-lib={}={}", ffmpeg_ty, lib.name);
        }
    }
    if env::var("CARGO_FEATURE_BUILD_ZLIB").is_ok() && cfg!(target_os = "linux") {
        println!("cargo:rustc-link-lib=z");
    }
}

fn main() {
    let statik = env::var("CARGO_FEATURE_STATIC").is_ok();
    let target_triple = env::var("TARGET").unwrap();

    let include_paths: Vec<PathBuf> = {
        let mut ffmpeg_dir = env::current_dir().unwrap();

        ffmpeg_dir.push("builds");
        ffmpeg_dir.push(target_triple);
        println!("cargo:rustc-link-search=native={}", ffmpeg_dir.join("lib").to_string_lossy());
        link_to_libraries(statik);
        vec![ffmpeg_dir.join("include")]
    };

    check_features(
        include_paths.clone(),
        &vec![
            ("libavutil/avutil.h", None, "FF_API_OLD_AVOPTIONS"),
            ("libavutil/avutil.h", None, "FF_API_PIX_FMT"),
            ("libavutil/avutil.h", None, "FF_API_CONTEXT_SIZE"),
            ("libavutil/avutil.h", None, "FF_API_PIX_FMT_DESC"),
            ("libavutil/avutil.h", None, "FF_API_AV_REVERSE"),
            ("libavutil/avutil.h", None, "FF_API_AUDIOCONVERT"),
            ("libavutil/avutil.h", None, "FF_API_CPU_FLAG_MMX2"),
            ("libavutil/avutil.h", None, "FF_API_LLS_PRIVATE"),
            ("libavutil/avutil.h", None, "FF_API_AVFRAME_LAVC"),
            ("libavutil/avutil.h", None, "FF_API_VDPAU"),
            ("libavutil/avutil.h", None, "FF_API_GET_CHANNEL_LAYOUT_COMPAT"),
            ("libavutil/avutil.h", None, "FF_API_XVMC"),
            ("libavutil/avutil.h", None, "FF_API_OPT_TYPE_METADATA"),
            ("libavutil/avutil.h", None, "FF_API_DLOG"),
            ("libavutil/avutil.h", None, "FF_API_HMAC"),
            ("libavutil/avutil.h", None, "FF_API_VAAPI"),
            ("libavutil/avutil.h", None, "FF_API_PKT_PTS"),
            ("libavutil/avutil.h", None, "FF_API_ERROR_FRAME"),
            ("libavutil/avutil.h", None, "FF_API_FRAME_QP"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_VIMA_DECODER"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_REQUEST_CHANNELS"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_OLD_DECODE_AUDIO"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_OLD_ENCODE_AUDIO"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_OLD_ENCODE_VIDEO"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_CODEC_ID"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_AUDIO_CONVERT"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_AVCODEC_RESAMPLE"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_DEINTERLACE"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_DESTRUCT_PACKET"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_GET_BUFFER"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_MISSING_SAMPLE"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_LOWRES"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_CAP_VDPAU"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_BUFS_VDPAU"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_VOXWARE"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_SET_DIMENSIONS"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_DEBUG_MV"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_AC_VLC"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_OLD_MSMPEG4"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_ASPECT_EXTENDED"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_THREAD_OPAQUE"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_CODEC_PKT"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_ARCH_ALPHA"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_ERROR_RATE"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_QSCALE_TYPE"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_MB_TYPE"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_MAX_BFRAMES"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_NEG_LINESIZES"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_EMU_EDGE"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_ARCH_SH4"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_ARCH_SPARC"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_UNUSED_MEMBERS"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_IDCT_XVIDMMX"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_INPUT_PRESERVED"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_NORMALIZE_AQP"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_GMC"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_MV0"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_CODEC_NAME"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_AFD"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_VISMV"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_DV_FRAME_PROFILE"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_AUDIOENC_DELAY"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_VAAPI_CONTEXT"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_AVCTX_TIMEBASE"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_MPV_OPT"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_STREAM_CODEC_TAG"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_QUANT_BIAS"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_RC_STRATEGY"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_CODED_FRAME"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_MOTION_EST"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_WITHOUT_PREFIX"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_CONVERGENCE_DURATION"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_PRIVATE_OPT"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_CODER_TYPE"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_RTP_CALLBACK"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_STAT_BITS"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_VBV_DELAY"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_SIDEDATA_ONLY_PKT"),
            ("libavcodec/avcodec.h", Some("avcodec"), "FF_API_AVPICTURE"),
            ("libavformat/avformat.h", Some("avformat"), "FF_API_LAVF_BITEXACT"),
            ("libavformat/avformat.h", Some("avformat"), "FF_API_LAVF_FRAC"),
            ("libavformat/avformat.h", Some("avformat"), "FF_API_URL_FEOF"),
            ("libavformat/avformat.h", Some("avformat"), "FF_API_PROBESIZE_32"),
            ("libavformat/avformat.h", Some("avformat"), "FF_API_LAVF_AVCTX"),
            ("libavformat/avformat.h", Some("avformat"), "FF_API_OLD_OPEN_CALLBACKS"),
            ("libavfilter/avfilter.h", Some("avfilter"), "FF_API_AVFILTERPAD_PUBLIC"),
            ("libavfilter/avfilter.h", Some("avfilter"), "FF_API_FOO_COUNT"),
            ("libavfilter/avfilter.h", Some("avfilter"), "FF_API_OLD_FILTER_OPTS"),
            ("libavfilter/avfilter.h", Some("avfilter"), "FF_API_OLD_FILTER_OPTS_ERROR"),
            ("libavfilter/avfilter.h", Some("avfilter"), "FF_API_AVFILTER_OPEN"),
            ("libavfilter/avfilter.h", Some("avfilter"), "FF_API_OLD_FILTER_REGISTER"),
            ("libavfilter/avfilter.h", Some("avfilter"), "FF_API_OLD_GRAPH_PARSE"),
            ("libavfilter/avfilter.h", Some("avfilter"), "FF_API_NOCONST_GET_NAME"),
            ("libavresample/avresample.h", Some("avresample"), "FF_API_RESAMPLE_CLOSE_OPEN"),
            ("libswscale/swscale.h", Some("swscale"), "FF_API_SWS_CPU_CAPS"),
            ("libswscale/swscale.h", Some("swscale"), "FF_API_ARCH_BFIN"),
        ],
    );
    // For debugging purpose only.
    let tmp = std::env::temp_dir();
    let mut f = File::create(tmp.join("ffmpeg4.build")).expect("Filed to create ffmpeg4.build");
    let tool = cc::Build::new().get_compiler();
    write!(f, "{}", tool.path().to_string_lossy().into_owned()).expect("failed to write cmd");
    for arg in tool.args() {
        write!(f, " {}", arg.to_str().unwrap()).expect("failed to write arg");
    }
    for dir in &include_paths {
        write!(f, " -I {}", dir.to_string_lossy().into_owned()).expect("failed to write incdir");
    }

    let clang_includes = include_paths.iter().map(|include| format!("-I{}", include.to_string_lossy()));

    // The bindgen::Builder is the main entry point
    // to bindgen, and lets you build up options for
    // the resulting bindings.
    let mut builder = bindgen::Builder::default()
        .clang_args(clang_includes)
        .ctypes_prefix("libc")
        // https://github.com/rust-lang/rust-bindgen/issues/550
        .blocklist_type("max_align_t")
        // Issue on aligned and packed struct. Related to:
        // https://github.com/rust-lang/rust-bindgen/issues/1538
        .opaque_type("__mingw_ldbl_type_t")
        // these are never part of ffmpeg API
        .blocklist_function("_.*")
        // Rust doesn't support long double, and bindgen can't skip it
        // https://github.com/rust-lang/rust-bindgen/issues/1549
        .blocklist_function("acoshl")
        .blocklist_function("acosl")
        .blocklist_function("asinhl")
        .blocklist_function("asinl")
        .blocklist_function("atan2l")
        .blocklist_function("atanhl")
        .blocklist_function("atanl")
        .blocklist_function("cbrtl")
        .blocklist_function("ceill")
        .blocklist_function("copysignl")
        .blocklist_function("coshl")
        .blocklist_function("cosl")
        .blocklist_function("dreml")
        .blocklist_function("ecvt_r")
        .blocklist_function("erfcl")
        .blocklist_function("erfl")
        .blocklist_function("exp2l")
        .blocklist_function("expl")
        .blocklist_function("expm1l")
        .blocklist_function("fabsl")
        .blocklist_function("fcvt_r")
        .blocklist_function("fdiml")
        .blocklist_function("finitel")
        .blocklist_function("floorl")
        .blocklist_function("fmal")
        .blocklist_function("fmaxl")
        .blocklist_function("fminl")
        .blocklist_function("fmodl")
        .blocklist_function("frexpl")
        .blocklist_function("gammal")
        .blocklist_function("hypotl")
        .blocklist_function("ilogbl")
        .blocklist_function("isinfl")
        .blocklist_function("isnanl")
        .blocklist_function("j0l")
        .blocklist_function("j1l")
        .blocklist_function("jnl")
        .blocklist_function("ldexpl")
        .blocklist_function("lgammal")
        .blocklist_function("lgammal_r")
        .blocklist_function("llrintl")
        .blocklist_function("llroundl")
        .blocklist_function("log10l")
        .blocklist_function("log1pl")
        .blocklist_function("log2l")
        .blocklist_function("logbl")
        .blocklist_function("logl")
        .blocklist_function("lrintl")
        .blocklist_function("lroundl")
        .blocklist_function("modfl")
        .blocklist_function("nanl")
        .blocklist_function("nearbyintl")
        .blocklist_function("nextafterl")
        .blocklist_function("nexttoward")
        .blocklist_function("nexttowardf")
        .blocklist_function("nexttowardl")
        .blocklist_function("powl")
        .blocklist_function("qecvt")
        .blocklist_function("qecvt_r")
        .blocklist_function("qfcvt")
        .blocklist_function("qfcvt_r")
        .blocklist_function("qgcvt")
        .blocklist_function("remainderl")
        .blocklist_function("remquol")
        .blocklist_function("rintl")
        .blocklist_function("roundl")
        .blocklist_function("scalbl")
        .blocklist_function("scalblnl")
        .blocklist_function("scalbnl")
        .blocklist_function("significandl")
        .blocklist_function("sinhl")
        .blocklist_function("sinl")
        .blocklist_function("sqrtl")
        .blocklist_function("strtold")
        .blocklist_function("tanhl")
        .blocklist_function("tanl")
        .blocklist_function("tgammal")
        .blocklist_function("truncl")
        .blocklist_function("y0l")
        .blocklist_function("y1l")
        .blocklist_function("ynl")
        .rustified_enum("*")
        .prepend_enum_name(false)
        .derive_eq(true)
        .size_t_is_usize(true)
        .parse_callbacks(Box::new(Callbacks));

    // The input headers we would like to generate
    // bindings for.
    if env::var("CARGO_FEATURE_AVCODEC").is_ok() {
        builder = builder
            .header(search_include(&include_paths, "libavcodec/avcodec.h"))
            .header(search_include(&include_paths, "libavcodec/dv_profile.h"))
            .header(search_include(&include_paths, "libavcodec/avfft.h"))
            .header(search_include(&include_paths, "libavcodec/vorbis_parser.h"));
    }

    if env::var("CARGO_FEATURE_AVDEVICE").is_ok() {
        builder = builder.header(search_include(&include_paths, "libavdevice/avdevice.h"));
    }

    if env::var("CARGO_FEATURE_AVFILTER").is_ok() {
        builder = builder
            .header(search_include(&include_paths, "libavfilter/buffersink.h"))
            .header(search_include(&include_paths, "libavfilter/buffersrc.h"))
            .header(search_include(&include_paths, "libavfilter/avfilter.h"));
    }

    if env::var("CARGO_FEATURE_AVFORMAT").is_ok() {
        builder = builder
            .header(search_include(&include_paths, "libavformat/avformat.h"))
            .header(search_include(&include_paths, "libavformat/avio.h"));
    }

    if env::var("CARGO_FEATURE_AVRESAMPLE").is_ok() {
        builder = builder.header(search_include(&include_paths, "libavresample/avresample.h"));
    }

    builder = builder
        .header(search_include(&include_paths, "libavutil/adler32.h"))
        .header(search_include(&include_paths, "libavutil/aes.h"))
        .header(search_include(&include_paths, "libavutil/audio_fifo.h"))
        .header(search_include(&include_paths, "libavutil/base64.h"))
        .header(search_include(&include_paths, "libavutil/blowfish.h"))
        .header(search_include(&include_paths, "libavutil/bprint.h"))
        .header(search_include(&include_paths, "libavutil/buffer.h"))
        .header(search_include(&include_paths, "libavutil/camellia.h"))
        .header(search_include(&include_paths, "libavutil/cast5.h"))
        .header(search_include(&include_paths, "libavutil/channel_layout.h"))
        .header(search_include(&include_paths, "libavutil/cpu.h"))
        .header(search_include(&include_paths, "libavutil/crc.h"))
        .header(search_include(&include_paths, "libavutil/dict.h"))
        .header(search_include(&include_paths, "libavutil/display.h"))
        .header(search_include(&include_paths, "libavutil/downmix_info.h"))
        .header(search_include(&include_paths, "libavutil/error.h"))
        .header(search_include(&include_paths, "libavutil/eval.h"))
        .header(search_include(&include_paths, "libavutil/fifo.h"))
        .header(search_include(&include_paths, "libavutil/file.h"))
        .header(search_include(&include_paths, "libavutil/frame.h"))
        .header(search_include(&include_paths, "libavutil/hash.h"))
        .header(search_include(&include_paths, "libavutil/hmac.h"))
        .header(search_include(&include_paths, "libavutil/imgutils.h"))
        .header(search_include(&include_paths, "libavutil/lfg.h"))
        .header(search_include(&include_paths, "libavutil/log.h"))
        // LZO is not "standalone" header. It's pulled as dependency of avcodec's
        //.header(search_include(&include_paths, "libavutil/lzo.h"))
        .header(search_include(&include_paths, "libavutil/macros.h"))
        .header(search_include(&include_paths, "libavutil/mathematics.h"))
        .header(search_include(&include_paths, "libavutil/md5.h"))
        .header(search_include(&include_paths, "libavutil/mem.h"))
        .header(search_include(&include_paths, "libavutil/motion_vector.h"))
        .header(search_include(&include_paths, "libavutil/murmur3.h"))
        .header(search_include(&include_paths, "libavutil/opt.h"))
        .header(search_include(&include_paths, "libavutil/parseutils.h"))
        .header(search_include(&include_paths, "libavutil/pixdesc.h"))
        .header(search_include(&include_paths, "libavutil/pixfmt.h"))
        .header(search_include(&include_paths, "libavutil/random_seed.h"))
        .header(search_include(&include_paths, "libavutil/rational.h"))
        .header(search_include(&include_paths, "libavutil/replaygain.h"))
        .header(search_include(&include_paths, "libavutil/ripemd.h"))
        .header(search_include(&include_paths, "libavutil/samplefmt.h"))
        .header(search_include(&include_paths, "libavutil/sha.h"))
        .header(search_include(&include_paths, "libavutil/sha512.h"))
        .header(search_include(&include_paths, "libavutil/stereo3d.h"))
        .header(search_include(&include_paths, "libavutil/avstring.h"))
        .header(search_include(&include_paths, "libavutil/threadmessage.h"))
        .header(search_include(&include_paths, "libavutil/time.h"))
        .header(search_include(&include_paths, "libavutil/timecode.h"))
        .header(search_include(&include_paths, "libavutil/twofish.h"))
        .header(search_include(&include_paths, "libavutil/avutil.h"))
        .header(search_include(&include_paths, "libavutil/xtea.h"))
        .header(search_include(&include_paths, "libavutil/hwcontext.h"));

    if env::var("CARGO_FEATURE_POSTPROC").is_ok() {
        builder = builder.header(search_include(&include_paths, "libpostproc/postprocess.h"));
    }

    if env::var("CARGO_FEATURE_SWRESAMPLE").is_ok() {
        builder = builder.header(search_include(&include_paths, "libswresample/swresample.h"));
    }

    if env::var("CARGO_FEATURE_SWSCALE").is_ok() {
        builder = builder.header(search_include(&include_paths, "libswscale/swscale.h"));
    }

    // Finish the builder and generate the bindings.
    let bindings = builder
        .generate()
        // Unwrap the Result and panic on failure.
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    bindings
        .write_to_file(output().join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
