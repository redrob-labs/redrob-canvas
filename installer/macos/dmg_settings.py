# SPDX-License-Identifier: MIT OR Apache-2.0
# dmgbuild settings for the Redrob Canvas disk image (installer/macos/make-dmg.sh passes the -D values).
#
# dmgbuild writes the Finder layout (.DS_Store) itself instead of scripting Finder, so the image comes
# out the same on a build machine reached over ssh and on a CI runner, neither of which can be relied
# on to let a script drive Finder. Icon centres and the window size match res/background.html.
import os.path

app = defines["app"]  # noqa: F821 -- dmgbuild injects `defines`
uninstaller = defines["uninstaller"]  # noqa: F821
here = defines["here"]  # noqa: F821

format = "UDZO"
compression_level = 9
filesystem = "HFS+"

files = [app, uninstaller]
# The licence texts and notices the zip carries beside the app, so the image ships them too.
licenses = defines.get("licenses")  # noqa: F821
if licenses:
    files.append((licenses, "Licenses"))
symlinks = {"Applications": "/Applications"}
icon = os.path.join(app, "Contents", "Resources", "AppIcon.icns")  # the volume's own icon

# dmgbuild picks up background@2x.png beside background.png and makes a Retina TIFF from the pair.
background = os.path.join(here, "res", "background.png")
window_rect = ((200, 120), (660, 400))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
arrange_by = None
icon_size = 96
text_size = 13
icon_locations = {
    os.path.basename(app): (170, 180),
    "Applications": (490, 180),
    os.path.basename(uninstaller): (250, 318),
    "Licenses": (410, 318),
}
