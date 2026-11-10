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
    if (!firstTarget.buildPhases.some(entry => entry.comment === phase)) {
      project.addBuildPhase([], "PBXShellScriptBuildPhase", phase, uuid, {
        shellPath: "/bin/sh",
        inputPaths: ["$(PODS_ROOT)/Target Support Files/Pods-$(TARGET_NAME)/Pods-$(TARGET_NAME)-acknowledgements.markdown"],
        outputPaths: ["$(TARGET_BUILD_DIR)/$(UNLOCALIZED_RESOURCES_FOLDER_PATH)/prns-cocoapods-notices.txt"],
        shellScript: 'set -eu\ntest -s "$SCRIPT_INPUT_FILE_0"\ncp "$SCRIPT_INPUT_FILE_0" "$SCRIPT_OUTPUT_FILE_0"',
      });
    }
    return configured;
  });
};
