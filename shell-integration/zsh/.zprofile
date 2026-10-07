__zhell_zdotdir=${ZDOTDIR}
ZDOTDIR=${ZHELL_USER_ZDOTDIR:-$HOME}
[[ -r "$ZDOTDIR/.zprofile" ]] && source "$ZDOTDIR/.zprofile"
ZDOTDIR=$__zhell_zdotdir
