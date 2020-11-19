#!/bin/bash
set -e
cargo build --bin tari_base_node
rm -fr temp/
npx cucumber-js --name "Simple propagation"
