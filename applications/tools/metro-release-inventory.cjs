// Record producer inputs before Expo serializes and the Hermes compiler omits
// modules with no mapped instructions (for example, asset stubs and type exports).
const { writeFileSync } = require('node:fs');
const path = require('node:path');

module.exports = function recordReleaseModules(config, directory) {
  const root = path.resolve(__dirname, '../..');
  const serializer = config.serializer.customSerializer;
  config.serializer.customSerializer = function (entry, preamble, graph, options) {
    const { platform, dev, minify, customTransformOptions } = graph.transformOptions;
    if (!['android', 'ios'].includes(platform) || dev !== false || minify !== true ||
        customTransformOptions?.engine !== 'hermes' || customTransformOptions.bytecode !== '1') {
      throw new Error('Expected a minified production mobile Hermes bundle');
    }
    const sources = [...new Set([...preamble.map(module => module.path), ...graph.dependencies.keys()])];
    const virtualContents = Object.fromEntries(preamble
      .filter(module => path.basename(module.path) === '__prelude__' || module.path.includes('\x00polyfill:'))
      .map(module => [path.relative(root, module.path), module.getSource().toString('utf8')]));
    writeFileSync(path.join(directory, platform + '.json'), JSON.stringify({
      platform, virtualContents, entry: path.relative(root, entry), sources: sources.map(file => path.relative(root, file)).sort(),
    }), { flag: 'wx' });
    return serializer.apply(this, arguments);
  };
};
