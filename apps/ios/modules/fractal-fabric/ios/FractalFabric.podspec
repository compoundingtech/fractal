Pod::Spec.new do |s|
  s.name = 'FractalFabric'
  s.version = '0.1.0'
  s.summary = 'Opt-in fabric gateway carrier'
  s.description = 'An app-owned iroh identity and loopback byte bridge: an opt-in carrier for the paired client gateway.'
  s.author = ''
  s.homepage = 'https://github.com/compoundingtech/fractal'
  s.license = 'MIT'
  s.platforms = { :ios => '16.4' }
  s.source = { git: '' }
  s.static_framework = true
  s.dependency 'ExpoModulesCore'
  # Preferred names mask their legacy counterparts even when empty or disabled.
  # Either carrier flag or proof flag can opt in to the same native bridge.
  if (ENV['FRACTAL_FABRIC'] || ENV['ST3_FABRIC']) == '1' ||
     (ENV['FRACTAL_FABRIC_PROOF'] || ENV['ST3_FABRIC_PROOF']) == '1'
    s.source_files = 'FractalFabricModule.swift'
    s.vendored_frameworks = 'build/FractalFabricRust.xcframework'
    s.frameworks = 'Security', 'SystemConfiguration', 'Network'
    s.libraries = 'resolv'
  else
    s.source_files = 'FractalFabricDisabled.swift'
  end
end
