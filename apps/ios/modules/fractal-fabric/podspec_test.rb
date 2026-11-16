require 'minitest/autorun'
require 'cocoapods'

class FractalFabricPodspecTest < Minitest::Test
  FLAGS = %w[FRACTAL_FABRIC ST3_FABRIC FRACTAL_FABRIC_PROOF ST3_FABRIC_PROOF].freeze
  PODSPEC = File.join(__dir__, 'ios', 'FractalFabric.podspec')

  def test_environment_precedence_and_native_build_names
    cases = [
      [{}, false],
      [{'FRACTAL_FABRIC' => '1'}, true],
      [{'ST3_FABRIC' => '1'}, true],
      [{'FRACTAL_FABRIC_PROOF' => '1'}, true],
      [{'ST3_FABRIC_PROOF' => '1'}, true],
      [{'FRACTAL_FABRIC' => '0', 'ST3_FABRIC' => '1'}, false],
      [{'FRACTAL_FABRIC' => '', 'ST3_FABRIC' => '1'}, false],
      [{'FRACTAL_FABRIC_PROOF' => '0', 'ST3_FABRIC_PROOF' => '1'}, false],
      [{'FRACTAL_FABRIC_PROOF' => '', 'ST3_FABRIC_PROOF' => '1'}, false],
      [{'FRACTAL_FABRIC' => '1', 'ST3_FABRIC' => '0'}, true],
      [{'FRACTAL_FABRIC_PROOF' => '1', 'ST3_FABRIC_PROOF' => '0'}, true],
      [{'FRACTAL_FABRIC' => '0', 'ST3_FABRIC_PROOF' => '1'}, true],
      [{'FRACTAL_FABRIC_PROOF' => '0', 'ST3_FABRIC' => '1'}, true]
    ]
    saved = FLAGS.to_h { |name| [name, ENV[name]] }
    begin
      cases.each do |environment, enabled|
        FLAGS.each { |name| ENV.delete(name) }
        ENV.update(environment)
        specification = Pod::Specification.from_file(PODSPEC).to_hash
        assert_equal 'FractalFabric', specification['name'], environment.inspect
        assert_equal(enabled ? 'FractalFabricModule.swift' : 'FractalFabricDisabled.swift',
                     specification['source_files'], environment.inspect)
        assert_equal(enabled ? 'build/FractalFabricRust.xcframework' : nil,
                     specification['vendored_frameworks'], environment.inspect)
      end
    ensure
      saved.each { |name, value| value.nil? ? ENV.delete(name) : ENV[name] = value }
    end
  end
end
