Pod::Spec.new do |s|
  s.name = 'FractalCollections'
  s.version = '0.1.0'
  s.summary = 'Fractal collections socket protocol heartbeat'
  s.description = 'Surfaces WebSocket ping events without an HTTP capability probe.'
  s.author = ''
  s.homepage = 'https://github.com/compoundingtech/fractal'
  s.license = 'MIT'
  s.platforms = { :ios => '15.1' }
  s.source = { git: '' }
  s.static_framework = true
  s.dependency 'ExpoModulesCore'
  s.dependency 'SocketRocket', '~> 0.7.1'
  s.source_files = '**/*.{h,m,swift}'
end
