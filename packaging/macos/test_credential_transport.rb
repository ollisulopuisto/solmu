# frozen_string_literal: true
require 'json'
require_relative 'credential_transport'
module NativePackages
  class Error < StandardError; end
end
class FakeSigning
  attr_reader :calls
  def initialize
    @calls = []
  end
  def root = Dir.pwd
  def execute(*args)
    @calls << [:fallback, args]
    'fallback'
  end
end
class FakeTransport < FakeSigning
  def secure_capture(argv, input, env)
    @calls << [argv, input, env]
    'safe'
  end
end

def check(value)
  raise 'Transport invariant failed' unless value
end

class SafeSigning < FakeTransport
  prepend SolmuMacosCredentialTransport
end
signer = SafeSigning.new
def signer.secure_capture(argv, input, env)
  @calls << [argv, input, env]
  "safe"
end
value = "dummy \"quotes\" \\ backslash $value"
signer.execute('security', 'import', 'certificate.p12', '-P', value, '-k', 'dummy keychain')
argv, input, env = signer.calls.last
check(argv == ['/usr/bin/security', '-i'])
check(!argv.join.include?(value))
check(input.include?('\\"quotes\\"'))
check(input.include?('\\\\ backslash'))
check(env.empty?)
%W[a\nb a\rb a\x00b].each do |bad|
  begin
    signer.execute('security', 'create-keychain', '-p', bad, 'dummy')
    raise 'Accepted command separator'
  rescue NativePackages::Error
  end
end
signer.execute('xcrun', 'notarytool', 'store-credentials', 'native-packages', '--keychain', 'dummy keychain', '--apple-id', 'dummy@example.invalid', '--team-id', 'DUMMYTEAM', '--password', 'dummy-apple')
argv, input, env = signer.calls.last
check(!argv.join.include?('dummy-apple'))
check(argv.last == 'store-notary-credentials')
check(env.fetch('APPLE_APP_PASSWORD') == 'dummy-apple')
check(env.fetch('KEYCHAIN_PATH') == 'dummy keychain')
check(env.fetch('NOTARY_PROFILE') == 'native-packages')
signer.execute('xcrun', 'notarytool', 'submit', 'dummy.dmg', '--keychain-profile', 'native-packages')
check(signer.calls.last.first == :fallback)
begin
  signer.execute('new-tool', '--password', 'dummy-password')
  raise 'Unsafe fallback accepted'
rescue NativePackages::Error
end
puts 'PASS: security stdin quoting, separators, notary profile transport and unsafe fallback'
