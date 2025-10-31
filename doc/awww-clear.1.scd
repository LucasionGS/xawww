awww-clear(1)

# NAME
awww-clear

# SYNOPSIS
*awww clear* [OPTIONS] <COLOR>

# OPTIONS

*-a*,*--all*
	Send this command to all active *awww-daemon* namespaces.

*-n*,*--namespace* <namespace>
	Which wayland namespace to send this command to.

	You can specify this option multiple times to send commands to multiple
	namespaces at once: *awww clear -n <namespace1> -n <namespace 2>*.

	This affects the name of the *awww-daemon* socket we will use to
	communicate with. Specifically, our socket name is
	_${WAYLAND_DISPLAY}-awww-daemon.<specified namespace>.socket_.

	Some compositors can have several different wallpapers per output. This
	allows you to differentiate between them. Most users will probably not have
	to set anything in this option.

*-o*, *--outputs*
	Comma separated list of outputs to display the image at. Use *awww query* to
	know which outputs are currently being used.

	If it isn't set, the image is displayed on all outputs.

*-h*, *--help*
	Print help (see a summary with '-h')

# COLOR

The color to fill the screen with. It must be given in *RRGGBB*, hex format. Note
there is no prepended '#'. Defaults to *000000*.

# DESCRIPTION
Fills the specified outputs with the given color.

Currently, we *do not* cache this, so if you want a color to be set at
initialization, you must set it every time:

```
awww-daemon && awww clear 1a804a
```

# SEE ALSO
*awww-query*(1)
