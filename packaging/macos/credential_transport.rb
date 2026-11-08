# frozen_string_literal: true

# Local transport adapter, not vendored dependency code. Keep the pinned gem's
# packaging/signing behavior, but intercept before it starts credential-bearing
# child processes. Remove after an upstream release provides safe transport.
require 'open3'
require 'pathname'

module SolmuMacosCredentialTransport
  PRIVATE_ENV = %w[APPLE_CERTIFICATE_P12 APPLE_CERTIFICATE_PASSWORD APPLE_APP_PASSWORD].freeze

  def execute(*arguments)
    args = arguments.map(&:to_s)
    if args.first == 'security'
      command = args.drop(1).map { |value| security_quote(value) }.join(' ') + "\n"
      return secure_capture(['/usr/bin/security', '-i'], command, {})
    end
    if args[0, 3] == ['xcrun', 'notarytool', 'store-credentials']
      options = args.drop(4).each_slice(2).to_h
      expected = %w[--keychain --apple-id --team-id --password]
      unless options.keys.sort == expected.sort && options.values.all? { |value| value && !value.empty? }
        raise NativePackages::Error, 'Unexpected notary credential options; refusing unsafe transport'
      end
      env = {
        'NOTARY_PROFILE' => args.fetch(3), 'KEYCHAIN_PATH' => options.fetch('--keychain'),
        'APPLE_ID' => options.fetch('--apple-id'), 'APPLE_TEAM_ID' => options.fetch('--team-id'),
        'APPLE_APP_PASSWORD' => options.fetch('--password')
      }
      helper = File.expand_path('macos_ci_credentials.py', __dir__)
      return secure_capture(['python3', helper, 'store-notary-credentials'], nil, env)
    end
    # Never fall through with a newly introduced password-bearing operation.
    if args.any? { |value| %w[--password -P].include?(value) }
      raise NativePackages::Error, 'Unsupported credential command; refusing argv transport'
    end
    super
  end

  private

  def security_quote(value)
    raise NativePackages::Error, 'Keychain input contains a line separator' if value.match?(/[\r\n\x00]/)
    '"' + value.gsub(/[\\"]/){ |character| '\\' + character } + '"'
  end

  def secure_capture(argv, input, additions)
    env = PRIVATE_ENV.to_h { |key| [key, nil] }.merge(additions)
    output, error, status = Open3.capture3(env, *argv, stdin_data: input.to_s, chdir: root.to_s)
    raise NativePackages::Error, 'Credential transport failed (output withheld)' unless status.success? && error.strip.empty?
    output.strip
  end
end
