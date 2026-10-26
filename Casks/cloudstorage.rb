cask "cloudstorage" do
  version "0.5.1"
  sha256 "a04ffba29d345fbad29cf86e31d8e2942a95f862c93b590ca987ea0e4fd26f87"

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
