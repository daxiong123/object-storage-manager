# 阿里云 OSS API 笔记

核对自官方「在 Header 中包含签名」与 GetService / GetBucket / PutObject / GetObject / DeleteObject。

| 操作 | 端点 | 签名 | 响应 |
|---|---|---|---|
| 列举空间 | `GET https://oss.aliyuncs.com/` | V1，`CanonicalizedResource=/` | XML `ListAllMyBucketsResult` |
| Bucket 区域 | `GET https://{bucket}.oss.aliyuncs.com/?location` | V1，`/{bucket}?location` | `LocationConstraint`，空 = `oss-cn-hangzhou` |
| 列举对象 | `GET https://{bucket}.{location}.aliyuncs.com/?delimiter=&marker=&max-keys=&prefix=` | V1，子资源按名字排序 | XML `ListBucketResult` |
| 下载 | `GET https://{bucket}.{location}.aliyuncs.com/{key}` | V1 | 对象流 |
| 上传 | `PUT` 同上，`Content-Type: application/octet-stream` | V1，StringToSign 含 Content-Type | 2xx |
| 删除 | `DELETE` 同上 | V1 | 2xx |
| 签名 URL | query：`OSSAccessKeyId` / `Expires` / `Signature` | StringToSign 的 Date 位换成 Expires 时间戳 | — |

注意：

- HMAC-SHA1 后用**标准 Base64**（带 padding），不是 URL-safe。
- Date 必须是 RFC 1123 英文（`Tue, 27 Dec 2011 03:37:58 GMT`），参与签名。
- 对象操作 virtual-hosted；本地 mock 走 path-style。
- `LastModified` 为 RFC 3339，Provider 换算为 Unix 毫秒。
