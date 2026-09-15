cask "cloudstorage" do
  version "0.2.0"
  sha256 "2b65bf77c84afbba7c6e158337979db6a15519b1b4b26278e6efc8e1057da7cf"

  url "https://github.com/daxiong123/object-storage-manager/releases/download/v#{version}/CloudStorage-#{version}-macos-arm64.zip",
      verified: "github.com/daxiong123/object-storage-manager/"
  name "CloudStorage"
  desc "跨云对象存储管理客户端（阿里云 OSS / 七牛云 / S3 兼容存储）"
  homepage "https://github.com/daxiong123/object-storage-manager"

depends_on macos: :sonoma

  # 当前产物为 ad-hoc 签名：首次安装后需右键打开一次，
  # 或改用 --no-quarantine 安装。后续正式发布将提供 Developer ID 签名与公证。
  app "CloudStorage.app"

  zap trash: [
    "~/Library/Application Support/CloudStorage",
    "~/Library/Preferences/com.example.cloudstorage.plist",
  ]
end
