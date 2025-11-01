use crate::wayland::zwlr_layer_shell_v1::Layer;
use common::ipc::PixelFormat;

pub struct Cli {
    pub format: Option<PixelFormat>,
    pub quiet: bool,
    pub no_cache: bool,
    pub layer: Layer,
    pub namespace: String,
}

impl Cli {
    pub fn new() -> Result<Option<Self>, CliError> {
        let mut quiet = false;
        let mut no_cache = false;
        let mut format = None;
        let mut layer = Layer::background;
        let mut namespace = String::new();
        let mut args = std::env::args();
        args.next(); // skip the first argument

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-f" | "--format" => match args.next().as_deref() {
                    Some("argb") => format = Some(PixelFormat::Argb),
                    Some("xrgb") => {
                        eprintln!(
                            "WARNING: xrgb is deprecated. Use `--format argb` instead.\n\
                            Note this is the default, so you can also just omit it."
                        );
                        format = Some(PixelFormat::Argb);
                    }
                    Some("abgr") => format = Some(PixelFormat::Abgr),
                    Some("rgb") => format = Some(PixelFormat::Rgb),
                    Some("bgr") => format = Some(PixelFormat::Bgr),
                    None => return Err(CliError::AbsentFormat),
                    Some(other) => return Err(CliError::UnrecognizedFormat(other.to_string())),
                },
                "-l" | "--layer" => match args.next().as_deref() {
                    Some("background") => layer = Layer::background,
                    Some("bottom") => layer = Layer::bottom,
                    None => return Err(CliError::AbsentLayer),
                    Some(other) => return Err(CliError::UnrecognizedLayer(other.to_string())),
                },
                "-n" | "--namespace" => {
                    namespace = match args.next() {
                        Some(s) => s,
                        None => return Err(CliError::AbsentNamespace),
                    }
                }
                "--no-cache" => no_cache = true,
                "-q" | "--quiet" => quiet = true,
                "-h" | "--help" => {
                    println!(
                        "\
awww-daemon

Options:

    -f|--format <argb|abgr|rgb|bgr>
        Force the use of a specific wl_shm format.

        By default, awww-daemon will use argb, because it is most widely
        supported. Generally speaking, formats with 3 channels will use 3/4 the
        memory of formats with 4 channels. Also, bgr formats are more efficient
        than rgb formats because we do not need to do an extra swap of the bytes
        when decoding the image (though the difference is unnoticiable).

    -l|--layer <background|bottom>
        Which layer to display the background in. Defaults to `background`.

        We do not accept layers `top` and `overlay` because those would make
        your desktop unusable by simply putting an image on top of everything
        else. If there is ever a use case for these, we can reconsider this.

    -n|--namespace <namespace>
        Which wayland namespace to append to `awww-daemon`.

        The resulting namespace will the `awww-daemon<specified namespace>`.
        This also affects the name of the `awww-daemon` socket we will use to
        communicate with the `client`. Specifically, our socket name is
        ${{WAYLAND_DISPLAY}}-awww-daemon.<specified namespace>.socket.

        Some compositors can have several different wallpapers per output. This
        allows you to differentiate between them. Most users will probably not have
        to set anything in this option.

    --no-cache
        Don't search the cache for the last wallpaper for each output.
        Useful if you always want to select which image 'awww' loads manually
        using 'awww img'.

    -q|--quiet    will only log errors
    -h|--help     print help
    -V|--version  print version"
                    );
                    return Ok(None);
                }
                "-V" | "--version" => {
                    println!("awww-daemon {}", env!("CARGO_PKG_VERSION"));
                    return Ok(None);
                }
                other => return Err(CliError::UnrecognizedArgument(other.to_string())),
            }
        }

        Ok(Some(Self {
            format,
            quiet,
            no_cache,
            layer,
            namespace,
        }))
    }
}

#[derive(Debug)]
pub enum CliError {
    AbsentFormat,
    UnrecognizedFormat(String),
    AbsentLayer,
    UnrecognizedLayer(String),
    AbsentNamespace,
    UnrecognizedArgument(String),
}

impl core::fmt::Display for CliError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CliError::AbsentFormat => f.write_str("format was not provided"),
            CliError::UnrecognizedFormat(format) => f.write_fmt(format_args!(
                "`--format` command line option must be one of: 'argb', 'abgr', 'rgb' or 'bgr'\n\
                Found: '{format}'"
            )),
            CliError::AbsentLayer => f.write_str("layer was not provided"),
            CliError::UnrecognizedLayer(layer) => f.write_fmt(format_args!(
                "`--layer` command line option must be one of: 'background', 'bottom'\n\
                Found: '{layer}'"
            )),
            CliError::AbsentNamespace => f.write_str("namespace was not provided"),
            CliError::UnrecognizedArgument(arg) => f.write_fmt(format_args!(
                "Unrecognized command line argument: {arg}\n\
                Run -h|--help to know what arguments are recognized!",
            )),
        }
    }
}

impl core::error::Error for CliError {}
