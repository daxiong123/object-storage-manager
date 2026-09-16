cask "cloudstorage" do
  version "0.3.0"
  sha256 "ffd7232410334ff285f1ba6d7a016eef03e1cb4584ae08e4fcd72e7d480d059c"

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
