let nixpkgs = import <nixpkgs> { };
in with nixpkgs;
stdenv.mkDerivation {
  name = "rust";
  buildInputs = [ rustup cargo-watch bintools clang ];
}
