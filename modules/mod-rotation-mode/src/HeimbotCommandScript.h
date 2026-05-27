#ifndef MOD_ROTATION_MODE_HEIMBOT_COMMAND_SCRIPT_H
#define MOD_ROTATION_MODE_HEIMBOT_COMMAND_SCRIPT_H

namespace RotationMode
{
    // Registers the .heimbot chat command tree:
    //
    //   .heimbot mode <off|rotation|grind|squad-mirror>
    //   .heimbot set <key> <value>
    //   .heimbot get <key>
    //   .heimbot list
    //   .heimbot reset
    //
    // Security: SEC_PLAYER (any logged-in character can use, scoped to
    // their own settings).
    //
    // v0 scaffold: subcommand handlers echo the action to the player's
    // chat but don't yet alter bot strategies. The DB store is wired so
    // values persist; the strategy-set application happens in step 3+.

    void RegisterCommandScript();
}

#endif // MOD_ROTATION_MODE_HEIMBOT_COMMAND_SCRIPT_H
