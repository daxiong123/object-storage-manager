cask "cloudstorage" do
  version "0.5.2"
  sha256 "4a41d2143abb0707b97ce65fa934ab49846db9b0d4145319da1db78e61e156d1"

  url "https://github.com/daxiong123/object-storage-manager/releases/download/v#{version}/CloudStorage-v#{version}-macos-arm64.zip"
  name "CloudStorage"
  desc "Native object storage manager for Qiniu Kodo, Aliyun OSS and Tencent COS"
  homepage "https://github.com/daxiong123/object-storage-manager"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "CloudStorage.app"

  caveats <<~EOS
    This release is ad-hoc signed and not notarized by Apple. If macOS blocks
    the first launch, allow CloudStorage in System Settings > Privacy & Security.
  EOS
end
