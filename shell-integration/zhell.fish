status is-interactive; or exit 0
set -q __zhell_loaded; and exit 0
set -g __zhell_loaded 1

function __zhell_escape
    string replace -a '\\' '\\\\' -- $argv[1] | string replace -a ';' '\\x3b' | string join '\\x0a'
end

function __zhell_preexec --on-event fish_preexec
    printf '\e]633;E;%s\a\e]133;C\a' (__zhell_escape "$argv[1]")
end

function __zhell_postexec --on-event fish_postexec
    printf '\e]133;D;%s\a' $status
end

function __zhell_cwd --on-variable PWD
    printf '\e]7;file://%s%s\a' $hostname (string escape --style=url -- $PWD)
end
__zhell_cwd

functions -q fish_prompt; and functions -c fish_prompt __zhell_user_prompt
function fish_prompt
    printf '\e]133;A\a'
    functions -q __zhell_user_prompt; and __zhell_user_prompt
    printf '\e]133;B\a'
end
