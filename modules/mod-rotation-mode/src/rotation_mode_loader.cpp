#include "RotationMode.h"

// AzerothCore's script loader looks for Addmod_<module_dir_name>Scripts.
// Our module dir is "mod-rotation-mode" so dashes become underscores.

void Addmod_rotation_modeScripts()
{
    AddRotationModeScripts();
}
