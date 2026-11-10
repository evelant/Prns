const path = require('node:path');
const { realpathSync } = require('node:fs');
const { getDefaultConfig } = require('expo/metro-config');

const config = getDefaultConfig(__dirname);
if (!config.resolver.assetExts.includes('txt')) config.resolver.assetExts.push('txt');
// This file-linked package lives outside the applications workspace. Metro can
// resolve it through node_modules without watching its rebuilt dist files.
// Watch its real package root, preserving Expo's workspace and resolver defaults.
const personalRns = path.dirname(realpathSync(require.resolve('personal-rns/package.json')));
require(path.join(personalRns, 'scripts/check-built-contracts.cjs')).checkBuiltContracts(personalRns);
config.watchFolders = [...new Set([...config.watchFolders, personalRns])];

if (process.env.PRNS_METRO_INVENTORY_DIR) {
  require('../../tools/metro-release-inventory.cjs')(config, process.env.PRNS_METRO_INVENTORY_DIR);
}
module.exports = config;
