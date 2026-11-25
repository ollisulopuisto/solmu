#!/usr/bin/env ruby
# frozen_string_literal: true

gem 'native-packages', '= 0.11.0'
require 'native_packages'
require 'native_packages/macos_signing'
require_relative 'credential_transport'
NativePackages::MacosSigning.prepend(SolmuMacosCredentialTransport)
NativePackages::CLI.run(Dir.pwd, ARGV)
