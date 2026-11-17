Pod::Spec.new do |s|
  s.name = 'FractalOutboxFiles'
  s.version = '0.1.0'
  s.summary = 'Private crash-recovery attachment storage'
  s.description = 'Backup-excluded private outbox directories and immutable attachment files.'
  s.author = ''
  s.homepage = 'https://github.com/compoundingtech/fractal'
  s.license = 'MIT'
  s.platforms = { :ios => '15.1' }
  s.source = { git: '' }
  s.static_framework = true
  s.dependency 'ExpoModulesCore'
  s.frameworks = 'Foundation', 'CryptoKit'
  s.source_files = '**/*.{h,m,swift}'
end
