cask "cloudstorage" do
  version "0.5.0"
  sha256 "4fd9ae9382d9f59fe43ac2a9cd317e58b62c895cb516744329037f2fba17d952"

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
