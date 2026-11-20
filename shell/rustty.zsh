# Source this LAST in ~/.zshrc (after starship/p10k): source /path/to/rustty/shell/rustty.zsh
# Wraps the prompt in an OSC 8 link "rustty:<exit status>" so rustty can find command blocks.
setopt prompt_subst
_rustty_st() { _rustty_s=$? }
_rustty_wrap() {
  _rustty_open=$'\e]8;;rustty:'$_rustty_s$'\e\\'
  _rustty_close=$'\e]8;;\e\\'
  [[ $PS1 == *_rustty_open* ]] || PS1='%{${_rustty_open}%}'$PS1'%{${_rustty_close}%}'
}
precmd_functions=(_rustty_st $precmd_functions _rustty_wrap)
