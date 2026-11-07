Pod::Spec.new do |s|
  s.name           = 'FractalPairingScanner'
  s.version        = '0.1.0'
  s.summary        = 'Pairing QR scanner for the Fractal app'
  s.description    = 'Presents a QR-only camera scanner without storing or logging the scan.'
  s.author         = ''
  s.homepage       = 'https://github.com/compoundingtech/fractal'
  s.license        = 'MIT'
  s.platforms      = { :ios => '15.1' }
  s.source         = { git: '' }
  s.static_framework = true
  s.dependency 'ExpoModulesCore'
  s.frameworks = 'AVFoundation', 'UIKit'
  s.source_files = '**/*.{h,m,swift}'
end
