awww-pause(1)

# NAME
awww-pause

# SYNOPSIS
*awww pause*

# OPTIONS

*-a*,*--all*
	Send this command to all active *awww-daemon* namespaces.

*-n*,*--namespace* <namespace>
	Which wayland namespace to send this command to.

	You can specify this option multiple times to send commands to multiple
	namespaces at once: *awww pause -n <namespace1> -n <namespace 2>*.

	This affects the name of the *awww-daemon* socket we will use to
	communicate with. Specifically, our socket name is
	_${WAYLAND_DISPLAY}-awww-daemon.<specified namespace>.socket_.

	Some compositors can have several different wallpapers per output. This
	allows you to differentiate between them. Most users will probably not have
	to set anything in this option.

*-h*, *--help*
	Print help (see a summary with '-h')

# DESCRIPTION

Toggles the pause state of the daemon. A animating gif will become frozen on the
last rendered frame until the command is run again.

Note that this will only pause the animation rendering of the daemon. It will
still respond to awww client commands.

# SEE ALSO
*awww-daemon*(1)
