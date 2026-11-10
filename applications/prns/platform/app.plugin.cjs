const { readFileSync } = require("node:fs");
const { resolve } = require("node:path");
const { withGradleProperties, withXcodeProject } = require("expo/config-plugins");

// Platform packaging only. Generated SDK namespaces and the selected native
// image are linked once by PrnsHostExpo / personal-rns-expo.
module.exports = function withPrnsAppPlatform(config) {
  config.ios = { ...config.ios, deploymentTarget: "18.0" };
  config.android = { ...config.android, allowBackup: false };
  config = withGradleProperties(config, configured => {
    configured.modResults = configured.modResults.filter(entry =>
      entry.type !== "property" || entry.key !== "android.minSdkVersion");
    configured.modResults.push({ type: "property", key: "android.minSdkVersion", value: "29" });
    return configured;
  });
  return withXcodeProject(config, configured => {
    const project = configured.modResults;
    const { uuid, firstTarget } = project.getFirstTarget();
    const phase = "Package CocoaPods acknowledgements";
    const repo = "$(SRCROOT)/../../../..";
    const modules = "$(SRCROOT)/../../../node_modules";
    const script = repo + "/applications/tools/native-bindings/ios_release_notices.py";
    const catalog = "release/licenses/app-ios/runtime-notices.json";
    const review = JSON.parse(readFileSync(resolve(__dirname, "../../..", catalog), "utf8"));
    const support = "$(PODS_ROOT)/Target Support Files/Pods-$(TARGET_NAME)/Pods-$(TARGET_NAME)";
    const inputs = [
      support + "-acknowledgements.markdown", support + "-acknowledgements.plist", support + "-frameworks.sh",
      "$(PODS_ROOT)/Manifest.lock", "$(SRCROOT)/Podfile.lock", repo + "/applications/package-lock.json",
      script, repo + "/" + catalog, ...Object.keys(review.inputs).map(file => modules + "/" + file),
      ...Object.entries(review.packages).flatMap(([name, item]) => [
        modules + "/" + name + "/package.json", ...(item.license_file ? [modules + "/" + name + "/" + item.license_file] : []),
      ]),
      ...review.supplemental_notices.map(item => repo + "/" + item.file),
    ];
    const phaseOptions = {
      shellPath: "/bin/sh",
      inputPaths: [...new Set(inputs)].map(input => JSON.stringify(input)),
      outputPaths: ['"$(TARGET_BUILD_DIR)/$(UNLOCALIZED_RESOURCES_FOLDER_PATH)/prns-cocoapods-notices.txt"'],
      shellScript: 'python3 "$SRCROOT/../../../tools/native-bindings/ios_release_notices.py" --acknowledgements "$SCRIPT_INPUT_FILE_0" --node-modules "$SRCROOT/../../../node_modules" --output "$SCRIPT_OUTPUT_FILE_0"',
    };
    const existing = firstTarget.buildPhases.find(entry => entry.comment === phase);
    if (existing) {
      const section = project.hash.project.objects.PBXShellScriptBuildPhase[existing.value];
      section.inputPaths = phaseOptions.inputPaths;
      section.outputPaths = phaseOptions.outputPaths;
      section.shellScript = JSON.stringify(phaseOptions.shellScript);
    } else {
      project.addBuildPhase([], "PBXShellScriptBuildPhase", phase, uuid, phaseOptions);
    }
    return configured;
  });
};
