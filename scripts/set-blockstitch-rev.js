const fs = require("fs");

const commit = process.argv[2];
if (!commit) {
  process.exit(0);
}

const cargo = fs.readFileSync("Cargo.toml", "utf8");
const next = cargo.replace(
  /(blockstitch-(?:core|qml) = \{ git = "https:\/\/github.com\/Blockworked\/blockstitch", rev = ")[^"]*(" \})/g,
  "$1" + commit + "$2",
);
if (next !== cargo) {
  fs.writeFileSync("Cargo.toml", next);
}