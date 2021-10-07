{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  buildInputs = with pkgs; let
    localPaths = [
      ".direnv"
      ".git"
      "shell.nix"
      "target"
    ];

    mkFlagList = name: lib.concatMapStringsSep " " (value: "${name} ${value}");
    excludeFlagList = mkFlagList "--exclude" localPaths;

    push = writeShellScriptBin "push" ''
      path=$(${coreutils}/bin/realpath --relative-to="$HOME" .)
      ${rsync}/bin/rsync -rhav --progress \
                         ${excludeFlagList} \
                         "$HOME/$path/" "''${1:-milan}.sev.lab.enarx.dev:$path"
    '';

    watch-push = writeShellScriptBin "watch-push" ''
      ${git}/bin/git ls-files | ${entr}/bin/entr -r '${push}/bin/push'
    '';
  in
  [
    rustup
    neovim

    push
    watch-push
  ];
}
