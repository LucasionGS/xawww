awww-query(1)

# NAME
awww-query

# SYNOPSIS
*awww query*

# OPTIONS

*-a*,*--all*
	Send this command to all active *awww-daemon* namespaces.

*-j*,*--j*
	Print the information in *json* format.

*-n*,*--namespace* <namespace>
	Which wayland namespace to send this command to.

	You can specify this option multiple times to send commands to multiple
	namespaces at once: *awww query -n <namespace1> -n <namespace 2>*.

	This affects the name of the *awww-daemon* socket we will use to
	communicate with. Specifically, our socket name is
	_${WAYLAND_DISPLAY}-awww-daemon.<specified namespace>.socket_.

	Some compositors can have several different wallpapers per output. This
	allows you to differentiate between them. Most users will probably not have
	to set anything in this option.

*-h*, *--help*
	Print help (see a summary with '-h')

# DESCRIPTION

Asks the daemon to print output information (names and dimensions).

You may use this to find out valid values for the <awww-img --outputs> option. If
you want more detailed information about your outputs, I would recommend trying
something like *wlr-randr*.

# OUTPUT FORMAT

Currently, *awww query* prints information in the following format:

```
NAMESPACE: OUTPUT: SIZE, scale: SCALE, currently displaying: IMAGE_OR_COLOR
```

where *SIZE* is in the format *WxH* (eg.: *1920x1080*), *SCALE* in "scale:
NUMBER", and *IMAGE_OR_COLOR* in
	- "image: IMAGENAME", if it's an image; or
	- "color: RGB", if it's a color

# JSON FORMAT

The *json* format will print information like so:

```
{
"NAMESPACE": [<LIST OF OUTPUTS>]
}
```

Where each output in the list is in format:

```
{
	"name": <NAME>,
	"width": <WIDTH>,
	"height": <HEIGHT>,
	"scale": <SCALE>,
	"displaying": {
		"image" OR "color": <PATH/TO/IMAGE> OR <COLOR IN HEXA>
	},
}
```
