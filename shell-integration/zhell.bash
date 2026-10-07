if [[ -n "${ZHELL_SHELL_LOGIN:-}" ]]; then
    unset ZHELL_SHELL_LOGIN
    [[ -r /etc/profile ]] && . /etc/profile
    for __zhell_f in ~/.bash_profile ~/.bash_login ~/.profile; do
        if [[ -r "$__zhell_f" ]]; then . "$__zhell_f"; break; fi
    done
    unset __zhell_f
else
    [[ -r /etc/bash.bashrc ]] && . /etc/bash.bashrc
    [[ -r ~/.bashrc ]] && . ~/.bashrc
fi

if [[ -n "${__zhell_loaded:-}" || $- != *i* ]] || (( BASH_VERSINFO[0] < 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] < 4) )); then
    return 0 2>/dev/null
fi
__zhell_loaded=1

__zhell_urlencode() {
    local s=$1 out= c i
    for (( i = 0; i < ${#s}; i++ )); do
        c=${s:i:1}
        case $c in
            [a-zA-Z0-9/._~-]) out+=$c ;;
            *) printf -v c '%%%02X' "'$c"; out+=$c ;;
        esac
    done
    printf '%s' "$out"
}

__zhell_escape() {
    local s=$1
    s=${s//\\/\\\\}
    s=${s//;/\\x3b}
    s=${s//$'\n'/\\x0a}
    s=${s//$'\r'/\\x0d}
    s=${s//$'\e'/\\x1b}
    s=${s//$'\a'/\\x07}
    printf '%s' "$s"
}

__zhell_precmd() {
    local ec=$?
    printf '\e]133;D;%s\a\e]7;file://%s%s\a' "$ec" "${HOSTNAME:-}" "$(__zhell_urlencode "$PWD")"
    return "$ec"
}

__zhell_ps1() {
    local ec=$?
    if [[ "$PS1" != *'133;A'* ]]; then
        PS1='\[\e]133;A\a\]'"$PS1"'\[\e]133;B\a\]'
    fi
    return "$ec"
}

__zhell_preexec() {
    local c
    c=$(HISTTIMEFORMAT= builtin history 1)
    [[ $c =~ ^[[:space:]]*[0-9]+\*?[[:space:]]+(.*)$ ]] && c=${BASH_REMATCH[1]}
    printf '\e]633;E;%s\a\e]133;C\a' "$(__zhell_escape "$c")"
}
PS0='$(__zhell_preexec)'"${PS0:-}"

if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
    PROMPT_COMMAND=(__zhell_precmd "${PROMPT_COMMAND[@]}" __zhell_ps1)
else
    PROMPT_COMMAND="__zhell_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND};__zhell_ps1"
fi
