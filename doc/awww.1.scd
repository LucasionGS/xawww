awww(1)

# NAME
awww - An Answer to your Wayland Wallpaper Woes

# SYNOPSIS
*awww* <COMMAND>

# COMMANDS

*clear*
	Fills the specified outputs with the given color

*restore*
	Restores the last displayed image on the specified outputs

*clear-cache*
	Deletes the *awww* cache directory

*img*
	Sends an image (or animated gif) for the daemon to display

*kill*
	Kills the daemon

*query*
	Asks the daemon to print output information (names and dimensions)

*help [COMMAND]*
	Print help or the help of the given command

# OPTIONS

*-h*, *--help*
	Print help (see a summary with '-h')

*-V*, *--version*
	Print  version

# DESCRIPTION
*awww* is a wallpaper manager that lets you change what your monitors display as
a background by controlling the *awww-daemon* at runtime.

It supports animated gifs and putting different stuff in different monitors. I
also did my best to make it as resource efficient as possible.

To start, begin by running *awww-daemon*. That will set up the *awww-daemon*.
Then, you can send images to be displayed with *awww img*. To kill the daemon,
use *awww kill*.

*Note that awww only works in a compositor that implements the layer-shell
protocol*. Typically, _wlr-roots_ based compositors.

# FILES
*awww* will create the following files in your system:
	- A socket in _${XDG_RUNTIME_DIR}/${WAYLAND_DISPLAY}-awww-daemon.<specified
	namespace>.socket_,  or
	_/tmp/awww/${WAYLAND_DISPLAY}-awww-daemon.<specified namespace>.socket_, if
	$XDG_RUNTIME_DIR is not set.
	- Cache files in _$XDG_CACHE_HOME/awww_ or _$HOME/.cache/awww_ if
	  $XDG_CACHE_HOME does not exist. These are used to set the wallpaper to the
	  previous image when a monitor is (re)connected or turned on.

# SEE ALSO
*awww-daemon*(1) *awww-clear*(1) *awww-img*(1) *awww-kill*(1) *awww-query*(1)
