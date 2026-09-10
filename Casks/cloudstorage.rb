cask "cloudstorage" do
  version "0.2.0"
  sha256 "f0116f2e599d8344b57d92b54dc8597efdae2f85c72208e2a0658b8c9f93c6a5"

  url "https://github.com/daxiong123/object-storage-manager/releases/download/v#{version}/CloudStorage-v#{version}-macos-arm64.zip"
  name "CloudStorage"
  desc "Native object storage manager for Qiniu Kodo and Aliyun OSS"
  homepage "https://github.com/daxiong123/object-storage-manager"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "CloudStorage.app"

  caveats <<~EOS
    This release is ad-hoc signed and not notarized by Apple. If macOS blocks
    the first launch, allow CloudStorage in System Settings > Privacy & Security.
  EOS
end
