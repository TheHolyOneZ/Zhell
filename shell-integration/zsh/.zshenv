__zhell_zdotdir=${ZDOTDIR}
ZDOTDIR=${ZHELL_USER_ZDOTDIR:-$HOME}
[[ -r "$ZDOTDIR/.zshenv" ]] && source "$ZDOTDIR/.zshenv"
ZDOTDIR=$__zhell_zdotdir
