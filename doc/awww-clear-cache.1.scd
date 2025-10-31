awww-clear-cache(1)

# NAME
awww-clear-cache

# SYNOPSIS
*awww clear-cache*

# OPTIONS

*-h*, *--help*
	Print help (see a summary with '-h')

# DESCRIPTION

Deletes the *awww* cache directory.

The cache resides at _$XDG_CACHE_HOME/awww_ or _$HOME/.cache/awww_
if $XDG_CACHE_HOME does not exist. In this directory, we create another
directory corresponding to *awww*s current version number. In there, we have
files that cache the last displayed image and animation frames for animated
wallpapers. There is one file per output.

Note that *awww* will automatically delete any files and directories in 
_$XDG_CACHE_HOME/awww_ that do not correspond to the exact current *awww*
version number. This is to ensure we only keep cache files for the latest
*awww* version.

# SEE ALSO
*awww-img*(1)
