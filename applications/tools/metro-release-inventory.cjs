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
    writeFileSync(path.join(directory, platform + '.json'), JSON.stringify({
      platform, entry: path.relative(root, entry), sources: sources.map(file => path.relative(root, file)).sort(),
    }), { flag: 'wx' });
    return serializer.apply(this, arguments);
  };
};
