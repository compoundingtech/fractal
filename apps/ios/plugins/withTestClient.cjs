const fs = require('node:fs');
const path = require('node:path');
const { withXcodeProject } = require('expo/config-plugins');

const bundle = 'com.compoundingtech.fractal.test';
const targetName = 'FractalTestControl';
const unquote = value => value?.replace(/^"|"$/g, '');

// This plugin is registered only by APP_VARIANT=test. Generated ios/ is disposable;
// the Swift observer is copied from release source, never patched after prebuild.
module.exports = function withTestClient(config) {
  if (config.ios?.bundleIdentifier !== bundle || config.extra?.testClientMode !== true) {
    throw new Error('withTestClient requires the isolated test identity');
  }
  return withXcodeProject(config, config => {
    const project = config.modResults;
    const objects = project.hash.project.objects;
    const app = project.getFirstTarget();
    const appName = unquote(app.firstTarget.name);
    const configurations = target => objects.XCConfigurationList[target.buildConfigurationList]
      .buildConfigurations.map(ref => objects.XCBuildConfiguration[ref.value].buildSettings);
    for (const settings of configurations(app.firstTarget)) {
      settings.CODE_SIGN_STYLE = 'Manual';
      settings.PROVISIONING_PROFILE_SPECIFIER = '"$(FRACTAL_IOS_TEST_APP_PROFILE)"';
    }

    let targetId = Object.keys(objects.PBXNativeTarget).find(id =>
      !id.endsWith('_comment') && unquote(objects.PBXNativeTarget[id].name) === targetName);
    if (!targetId) {
      targetId = project.generateUuid();
      const settings = {
        PRODUCT_BUNDLE_IDENTIFIER: bundle,
        PRODUCT_NAME: '"$(TARGET_NAME)"',
        GENERATE_INFOPLIST_FILE: 'YES',
        SWIFT_VERSION: '5.0',
        IPHONEOS_DEPLOYMENT_TARGET: configurations(app.firstTarget)[0].IPHONEOS_DEPLOYMENT_TARGET ?? '15.1',
        TARGETED_DEVICE_FAMILY: '"1,2"',
        SDKROOT: 'iphoneos',
        CODE_SIGN_STYLE: 'Manual',
        PROVISIONING_PROFILE_SPECIFIER: '"$(FRACTAL_IOS_TEST_RUNNER_PROFILE)"',
        // Xcode appends .xctrunner to PRODUCT_BUNDLE_IDENTIFIER for the UI runner.
        // Do not give the runner any app-group or keychain-sharing entitlements.
        TEST_TARGET_NAME: `"${appName}"`,
        SKIP_INSTALL: 'YES',
        LD_RUNPATH_SEARCH_PATHS: '"$(inherited) @executable_path/Frameworks @loader_path/Frameworks"',
      };
      const list = project.addXCConfigurationList(['Debug', 'Release'].map(name => ({
        isa: 'XCBuildConfiguration', name, buildSettings: { ...settings },
      })), 'Release', `Build configuration list for PBXNativeTarget "${targetName}"`);
      const product = project.addProductFile(`${targetName}.xctest`, {
        target: targetId, sourceTree: 'BUILT_PRODUCTS_DIR',
      });
      // Supplying explicitFileType to addProductFile appends the library's first
      // cfbundle extension (.mdimporter). Set the type after the exact path exists.
      objects.PBXFileReference[product.fileRef].explicitFileType = '"wrapper.cfbundle"';
      const nativeTarget = {
        isa: 'PBXNativeTarget', name: targetName, productName: targetName,
        productReference: product.fileRef,
        productType: '"com.apple.product-type.bundle.ui-testing"',
        buildConfigurationList: list.uuid, buildPhases: [], buildRules: [], dependencies: [],
      };
      const target = { uuid: targetId, pbxNativeTarget: nativeTarget };
      project.addToPbxNativeTargetSection(target);
      project.addToPbxProjectSection(target);
      project.addTargetDependency(targetId, [app.uuid]);
      project.addBuildPhase([], 'PBXSourcesBuildPhase', 'Sources', targetId);
      project.addBuildPhase([], 'PBXFrameworksBuildPhase', 'Frameworks', targetId);
      project.addBuildPhase([], 'PBXResourcesBuildPhase', 'Resources', targetId);
      const group = project.addPbxGroup([], targetName, targetName);
      const root = project.getFirstProject().firstProject;
      project.addToPbxGroup(group.uuid, root.mainGroup);
      project.addSourceFile('ClientControlUITests.swift', { target: targetId }, group.uuid);
    }
    const attributes = project.getFirstProject().firstProject.attributes;
    attributes.TargetAttributes ??= {};
    attributes.TargetAttributes[targetId] = { TestTargetID: app.uuid, ProvisioningStyle: 'Manual' };
    attributes.TargetAttributes[app.uuid] = {
      ...attributes.TargetAttributes[app.uuid], ProvisioningStyle: 'Manual',
    };

    const nativeRoot = config.modRequest.platformProjectRoot;
    fs.mkdirSync(path.join(nativeRoot, targetName), { recursive: true });
    fs.copyFileSync(path.join(__dirname, 'ClientControlUITests.swift'),
      path.join(nativeRoot, targetName, 'ClientControlUITests.swift'));
    const schemeRoot = path.join(nativeRoot, `${appName}.xcodeproj`, 'xcshareddata', 'xcschemes');
    fs.mkdirSync(schemeRoot, { recursive: true });
    const reference = (id, name, product) => `<BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="${id}" BuildableName="${product}" BlueprintName="${name}" ReferencedContainer="container:${appName}.xcodeproj"/>`;
    const appRef = reference(app.uuid, appName, `${appName}.app`);
    const testRef = reference(targetId, targetName, `${targetName}.xctest`);
    fs.writeFileSync(path.join(schemeRoot, `${targetName}.xcscheme`), `<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion="1600" version="1.3">
  <BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES"><BuildActionEntries>
    <BuildActionEntry buildForTesting="YES" buildForRunning="YES" buildForProfiling="NO" buildForArchiving="NO" buildForAnalyzing="YES">${appRef}</BuildActionEntry>
    <BuildActionEntry buildForTesting="YES" buildForRunning="NO" buildForProfiling="NO" buildForArchiving="NO" buildForAnalyzing="YES">${testRef}</BuildActionEntry>
  </BuildActionEntries></BuildAction>
  <TestAction buildConfiguration="Release" shouldUseLaunchSchemeArgsEnv="NO"><Testables><TestableReference skipped="NO" parallelizable="NO">${testRef}</TestableReference></Testables></TestAction>
  <LaunchAction buildConfiguration="Release" launchStyle="0" useCustomWorkingDirectory="NO" ignoresPersistentStateOnLaunch="NO" debugServiceExtension="internal" allowLocationSimulation="NO"><BuildableProductRunnable runnableDebuggingMode="0">${appRef}</BuildableProductRunnable></LaunchAction>
  <ProfileAction buildConfiguration="Release" shouldUseLaunchSchemeArgsEnv="YES" useCustomWorkingDirectory="NO"><BuildableProductRunnable runnableDebuggingMode="0">${appRef}</BuildableProductRunnable></ProfileAction>
  <AnalyzeAction buildConfiguration="Release"/>
  <ArchiveAction buildConfiguration="Release" revealArchiveInOrganizer="YES"/>
</Scheme>
`);
    return config;
  });
};
