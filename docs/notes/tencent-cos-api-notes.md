# 腾讯云 COS API 笔记（在源码里核实过的部分）

> 本文只写**已核实**的事实与踩坑。签名部分逐字节对照官方文档 + 五个官方 SDK
> （Go / Python / Node / Java / JS）得出，并内置了三条可复现的测试向量
> （`crates/provider-tencent/src/sign.rs`）。
>
> **腾讯云没有官方 Rust SDK**（`github.com/tencentyun` 组织下只有 c/cpp/java/go/python/node/js/php
> 等，没有 rust），所以只能照规范实现，没有可对照的参考实现。

## 0. 官方文档入口

中文文档是客户端渲染的 SPA，`curl` 只拿到壳；国际站镜像（`intl.cloud.tencent.com`）
是服务端渲染的同一份文档，可以直接抓。签名算法文档是 `product/436/7778`。

## 1. 签名 v5（`q-sign-algorithm=sha1`）

### 1.1 派生链

```text
KeyTime      = [StartUnix];[EndUnix]
SignKey      = hex(HMAC-SHA1(SecretKey, KeyTime))
HttpString   = [method 小写]\n[UriPathname]\n[HttpParameters]\n[HttpHeaders]\n
StringToSign = "sha1"\n[KeyTime]\nhex(SHA1(HttpString))\n
Signature    = hex(HMAC-SHA1(SignKey, StringToSign))
```

Authorization 头的值（七个键，顺序固定，`&` 连接）：

```text
q-sign-algorithm=sha1&q-ak=<SecretId>&q-sign-time=<KeyTime>&q-key-time=<KeyTime>&q-header-list=<HeaderList>&q-url-param-list=<UrlParamList>&q-signature=<Signature>
```

`q-sign-time` 与 `q-key-time` 在**所有五个 SDK 里都是同一个 KeyTime 串**。

### 1.2 四个必须按原样复现、否则静默签错的点

1. **SignKey 的十六进制字符串是当「文本」用的，不是当原始字节。**
   它是下一层 HMAC 的 key：`hmac(SignKey_bytes_of_the_HEX_STRING, StringToSign)`。
   官方文档原文：*"Use HMAC-SHA1 with SignKey as the key (in string form, not raw binary)"*。
   五个 SDK 一致（Go 先 `fmt.Sprintf("%x", digest)` 再把该串喂给 `hmac.New`）。
   写成 `hmac(sign_key_raw_bytes, ...)` 会得到一个服务端永远算不出的签名。

2. **`UriPathname` 是解码后的原始 UTF-8 路径，不是线上的百分号编码形式。**
   官方文档例 1 的请求行是 `PUT /exampleobject(%E8%85%BE%E8%AE%AF%E4%BA%91)`，
   但**只有**用解码后的 `腾讯云` 才能算出文档自己公布的那个
   `SHA1(HttpString) = 8b2751e77f43a0995d6e9eb9477f4b685cca4172`。
   本仓库用 `http_string_uses_decoded_path` 把这条钉死（用线上编码形式会红）。

3. **`HttpString` 用 LF（`\n`），结尾那个 `\n` 必须有；空分量保留成空行。**
   文档原文：*"If any string within it is empty, the preceding and following newline
   characters must be retained, for example, `get\n/exampleobject\n\n\n`"*。
   Java SDK 里直接写着 `LINE_SEPARATOR = "\n"`。

4. **签名用的头是「按需挑选的子集」。** 我们只签 `host`（上传再加 `content-type`），
   不签 `date` / `content-length` / `content-md5`。签得越少，越不容易因为值与实际
   发送的不一致而炸；代价是这几个头失去防篡改保护，对本地客户端无实际影响。
   **`host` 必须参与签名**（缺了它服务端算不出相同 HttpString；Java SDK 直接抛
   `buildAuthorization missing header: host`）。

### 1.3 UrlEncode 规则

等价于 JS `encodeURIComponent`，**再额外编码 `! ' ( ) *` 五个字符**，
百分号用**大写**十六进制，空格 → `%20`（**绝不** `+`），保留 `A-Za-z0-9-_.~`。

- key 与 value 都要编码；key 编码后还要转小写。
- 排序按「编码并小写后的 key」字典序。
- `HeaderList` / `UrlParamList` 用 `;` 连接；`HttpHeaders` / `HttpParameters` 用 `&` 连接 `key=value`。
- 无值参数按空串处理（`/?acl` → `acl=`）。

### 1.4 三条内置测试向量

| 向量 | 出处 | 验证什么 |
|---|---|---|
| `SHA1(HttpString) = 8b2751e77f43a0995d6e9eb9477f4b685cca4172` | 官文档例 1（上传） | 路径必须解码；头名排序；值编码 |
| `SHA1(HttpString) = 54ecfe22f59d3514fdc764b87a32d8133ea611e6` | 官文档例 2（下载 + `response-*`） | query 参数排序；值内 `=` 编码成 `%3D` |
| `q-signature = ce4ac0ecbcdb30538b3fee0a97cc6389694ce53a` | Go SDK `auth_test.go` | **端到端**：SignKey 当文本、空 `q-url-param-list`、方法小写 |

前两条不需要 SecretKey（文档里的密钥被打码），因此能独立锁死 HttpString 的构造。

> ⚠️ 官方文档两条例子里**最终 Signature 的末几位是脱敏伪造的**：前 35 位与实际
> 计算一致，尾 5 位被换成了 `1234` 之类的填充。所以断言要用实算值，不要抄文档展示值。
> 文档例 1 的实际签名是 `3b8851a11a569213c17ba8fa7dcf2abec6935172`（文档印的是 `...6931234`）。

### 1.5 预签名 URL

**同一套算法**，只是把七个 `q-*` 参数从 `Authorization` 头挪到 query 串：
把整条 Authorization 做百分号编码，但 **`&` 与 `=` 保持字面量**（否则 query 解析不出
七个参数），于是 `q-sign-time` 里的 `;` 变成 `%3B`。

签名时的 `HttpParameters` 只包含**真实业务 query**（本项目的 `signed_get_url` 为空），
`q-*` 那七个是签名之后才拼上去的，不参与 HttpString。HeaderList 只有 `host`。

`x-cos-security-token`：仅在使用临时（STS）凭证时需要，本项目暂不支持 STS。

### 1.6 时间与错误

- KeyTime 是 Unix **秒**（无填充、无毫秒），格式 `Start;End`。
- 请求落在窗口外 → `403 Request has expired`；本地时钟与服务器差超过
  **15 分钟** → `403 RequestTimeTooSkewed`。
- 本项目请求头签名的有效期取 30 分钟（`REQUEST_SIGNATURE_TTL`）。

## 2. 端点与请求结构

| 场景 | Host |
|---|---|
| 列举全部存储桶 | `service.cos.myqcloud.com`（`GET /`，返回桶跨地域，带 `<Location>`） |
| 指定地域列举 | `cos.<Region>.myqcloud.com`，或全局入口加 `?region=<Region>` |
| 对象操作（推荐） | `{BucketName-APPID}.cos.{Region}.myqcloud.com`（virtual-hosted） |
| 对象操作（路径型） | `cos.<Region>.myqcloud.com/{BucketName-APPID}/{Key}` |

### 2.1 踩坑：Bucket 必须带 APPID 后缀

存储桶标识是 `<BucketName>-<APPID>`（如 `examplebucket-1250000000`），
名称段只允许 `[a-z0-9-]`，APPID 段是纯数字，完整请求域名 ≤ 60 字符。
Go SDK 的正则是 `^[a-z0-9-]+-[0-9]+$`。

只写名称会得到 DNS 解析失败或 404，**看不出原因**，所以
`crates/provider-tencent` 在联网前用 `bucket_name_error` 挡住并给出可操作提示。

### 2.2 踩坑：ListBuckets **也会分页**

容易被当成「没有分页」的接口。响应里同样有顶层 `Marker` / `NextMarker` /
`IsTruncated`，桶多时要循环取完（`list_buckets_follows_pagination` 锁死这条）。

## 3. 对象 API 要点

### ListObjects（`GET /?prefix=&delimiter=&marker=&max-keys=`）

- 响应根 `ListBucketResult`，元素：`Name` / `Prefix` / `Marker` / `NextMarker` /
  `MaxKeys` / `Delimiter` / `IsTruncated` / `EncodingType` / `Contents{Key,LastModified,ETag,Size,StorageClass,Owner}` /
  `CommonPrefixes{Prefix}`。
- `max-keys`：默认 1000，范围 0..=1000。
- `LastModified` 是 ISO8601，**秒或带毫秒两种都出现过**（`2019-05-24T10:56:40Z` 与
  `2020-12-10T03:37:30.000Z`），解析要都能吃下。
- `NextMarker`：文档明确「仅当 `IsTruncated` 为 true 才返回」，值是本页最后一个对象键。
  本项目仍保留「截断但没给 NextMarker 就从最后一项兜底合成」的分支（与阿里云同款防御）。
- `encoding-type=url` 可选。本项目**不发**它，与七牛/阿里云一致地拿原始 key。
- 文档没有写明 `delimiter` 把 key 归并进 `CommonPrefixes` 时 `NextMarker` 取值如何，
  如果依赖 delimiter 翻页需要实测确认。

### PUT Object（上传）

- `PUT /<ObjectKey>`，成功响应体为空，只有 `ETag` 等响应头。
- **简单上传上限 5 GB**，超过必须走分块上传。本项目明确不做分块上传，
  超过即在联网前 Fail Fast 报错（`MAX_SIMPLE_UPLOAD_BYTES`）。
- 支持 `Transfer-Encoding: chunked`（此时**不能**再给 `Content-Length`），
  所以本项目流式上传走 chunked，不预先声明长度。
- `Content-Type` 文档标为必填；本项目固定 `application/octet-stream` 并把它签进去。

### GET / DELETE Object

- `GET /<ObjectKey>` 支持 `Range`（只支持单范围）；`response-*` 是 **query 参数**不是头。
- 对象不存在 → `404 NoSuchKey`；桶不存在 → `404 NoSuchBucket`。
- `DELETE /<ObjectKey>` 成功返回 **204**；**删不存在的 key 也算成功**（幂等）。

## 4. 错误码映射（与阿里云不同，不能照抄）

错误响应是 XML：`<Error><Code><Message><Resource><RequestId><TraceId></Error>`。
`RequestId` 也可能出现在响应头 `x-cos-request-id`。

| COS | HTTP | 本项目映射 |
|---|---|---|
| `SignatureDoesNotMatch` / `InvalidAccessKeyId` | 403 | `Auth`（附 `RequestId`，这是排查 COS 签名问题时唯一能拿给服务端的凭据） |
| `AccessDenied` | 403 | `Api`；`list_buckets` 场景改报「无 `cos:GetService` 权限，请手填桶名」 |
| `NoSuchBucket` / `NoSuchKey` | 404 | `Api` + 可操作文案 |
| `SlowDown` | **503** | `RateLimited` |

> ⚠️ **COS 核心对象 API 的限流是 503 `SlowDown`，不是 429。** 核心文档里查不到
> 429（429 `TooManyRequestsException` 只出现在另一套 data API 里）。阿里云那套
> 把 503/429 一起判限流，这里按 COS 的真实码来。

## 5. 目录语义

COS 没有真实目录。在控制台「新建文件夹」实际是**创建一个键以 `/` 结尾、内容为空的
对象**（如 `project/`），靠展示方式模拟文件夹。**没有独立的建目录 API**——
上传一个空对象即可。删除文件夹对象**不会**删除其子对象。

因此目录语义的唯一载体仍是 `delimiter` 列举返回的 `CommonPrefixes`；
占位对象要在数据填充点单点过滤掉（`crates/ui/src/workspace/objects.rs` 的
`is_directory_object`，判据是 key 以 `/` 结尾），与七牛的
`application/qiniu-object-manager` 是同一类东西。

## 6. 地域

地域是静态文档列表，**没有「列举全部地域」的 API**：
`ap-beijing` / `ap-nanjing` / `ap-shanghai` / `ap-guangzhou` / `ap-chengdu` /
`ap-chongqing` / `ap-hongkong` / `ap-singapore` / `ap-jakarta` / `ap-seoul` /
`ap-bangkok` / `ap-tokyo` / `ap-osaka` / `na-siliconvalley` / `na-ashburn` /
`sa-saopaulo` / `eu-frankfurt` / `me-saudi-arabia` 等。

单个桶的地域：`GET /?location` → `<LocationConstraint>ap-beijing</LocationConstraint>`，
但**这个请求本身就需要桶域名，而桶域名需要地域**——所以它不能用来「反推」地域。

本项目的地域来源是 ListBuckets 返回的 `<Location>`（全局入口一次就能拿到所有桶的地域），
UI 把它随手填进 `ListObjectsRequest::region`。缺失时（只有「手填桶名」那条路径会缺）
回退查一次全局入口并缓存，仍拿不到才报错——**不蒙默认地域**：地域错了服务端只会回
`SignatureDoesNotMatch`，用户完全无从判断。
