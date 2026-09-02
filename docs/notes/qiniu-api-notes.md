# 七牛 API 备忘（逐字节核对自官方 SDK 源码）

> 核对基准：qiniu-credential 0.2.4 / qiniu-http-client 0.2.4 / qiniu-apis 0.2.4 / qiniu-utils 0.2.4 官方源码。
> 实现位置：`crates/provider-qiniu/src/sign.rs` + `crates/provider-qiniu/src/lib.rs`。
> 官方测试向量已内置为单元测试（`sign_token_matches_official_vector_*` 等），改动签名代码必须保持这些测试通过。

## 1. 签名算法（V2 请求签名）

sign 数据（对应官方 `_sign_request_v2_without_body` + Content-Type 分支）：

```
METHOD path[?query]\n
Host: host[:port]\n
[Content-Type: value\n]
[X-Qiniu-Name: value\n]*    ← 多个时排序
\n                          ← 最后一个空行永远存在
```

- HMAC-SHA1，key = SecretKey；结果 Base64 **URL_SAFE 带 padding**（`-_` 字母表 + 尾部 `=`）。
  官方 `qiniu-utils::base64::urlsafe` 用的是 `base64::URL_SAFE`（padded），不是 NO_PAD —— 见官方向量尾部 `=`。
- Authorization 头：`Qiniu AK:sig`。
- **签名必须用实际发送的原始 query 串**。reqwest 发送 `Url` 的 query 原文；因此本项目的顺序是：
  先 `url.set_query(...)`，再取 `url.query()` 参与签名。绝不要"先签名后拼 URL"。
- X-Qiniu-* 头参与签名前要**规范化名称**（官方 `make_header_name`：每个 `-` 后首字母大写，
  其余小写，即 `x-qiniu-aaa` → `X-Qiniu-Aaa`），**规范化后**再过滤（裸 `X-Qiniu-` 长度 ≤ 8 被排除）
  和按 (name, value) 排序。
- Content-Type 行：有 body 时参与（值原样，不规范化）。GET 无 body 请求不写这行。
- 老版 V1（`QBox AK:sig`）：sign 数据 = `path[?query]\n`，仅 form 表单 POST 时附加 body。
  当前未实现（无调用方），需要时基于 `sign_token` 两行拼出。

## 2. 官方测试向量（已内置为单元测试）

- V1：`Credential::new("abcdefghklmnopq","1234567890").sign(b"hello")`
  = `abcdefghklmnopq:b84KVc-LroDiz0ebUANfdzSRxa0=`；`sign(b"world")` = `...:VjgXt0P_nCxHuaTfiFz-UjDJ1AQ=`
- V2：GET `http://upload.qiniup.com/mkfile/sdf.jpg`，headers `x-qiniu-bbb: AAA`、`x-qiniu-aaa: CCC`，
  `Credential::new("ak","sk")` → `Qiniu ak:arPKqUn6T6DrnHhygbFS40PGBgY=`

## 3. 管理 API / 上传（已实现）

| 操作 | 端点 | 鉴权 | 返回 |
|---|---|---|---|
| 列举空间 | UC `GET https://uc.qiniuapi.com/buckets` | V2 | bucket 名字符串数组 |
| 列举文件 | RSF `GET https://rsf.qbox.me/list?bucket=&marker=&limit=&prefix=&delimiter=` | V2 | `{marker, items[], commonPrefixes[]}` |
| 表单上传 | Up `POST https://upload.qiniup.com/`（默认华东/智能；区域域名后续按 UC `/v4/query`） | **上传凭证**（非 V2 请求签名） | `{hash,key}` |
| 删除对象 | RS `POST https://rs.qbox.me/delete/<urlsafe_base64(bucket:key)>` | V2，无 body | 空 JSON `{}` |

### 上传凭证（官方 `Credential::sign_with_data`）

- policy JSON：`{"scope":"bucket:key","deadline":<unix>}`（serde 序列化，key 内特殊字符走 JSON 转义）
- token = `AK:sign(base64(policy)):base64(policy)`，HMAC-SHA1 + URL_SAFE **带 padding**
- 官方向量已内置：`sign_with_data(b"hello")` = `abcdefghklmnopq:BZYt5uVRy1RVt5ZTXbaIt2ROVMA=:aGVsbG8=`
- 表单字段：`token` / `key` / `file`（流式 Part，已知 Content-Length）。**不要**再带 `Authorization` 头。

- `limit` 合法范围 **1–1000**；provider 校验超出报 `InvalidInput`。
- `items[]` 元素：`key, hash, fsize, mimeType, putTime, type, status`。
  **putTime 单位是 100ns**（Unix epoch 起），换算毫秒 = `putTime / 10_000`。
- `commonPrefixes` 是 delimiter 列举产生的模拟目录前缀（以 `/` 结尾）。
- marker 为空字符串或缺失 = 列举结束（`ObjectPage::has_more()` = false）。
- query 值编码用 RFC 3986（`/` → `%2F`，空格 → `%20`），不用 form-urlencoded 的 `+` 空格，
  规避服务端解码歧义。编码后字符串同时用于签名与发送，两者必然一致。

## 4. 错误响应

- 非 2xx 响应体为 JSON `{"error": "..."}`。
- 七牛使用**非标准 HTTP 状态码**：631（空间不存在）、612（文件不存在）、573/579（限流）等。
  `StorageError` 映射：401 → `Auth`；429/573/579 → `RateLimited`；其余（含 6xx）→ `Api { status, message }`。

## 5. 测试

- 单元测试含本地 TCP mock（一次性 accept），断言"实际发送的请求行/Host/Authorization"
  与用同一签名函数对同一原始串重算的结果一致——验证的是**签名数据构造**层；
  HMAC 原语正确性由官方向量测试保证。
- 真实凭证冒烟：`QINIU_ACCESS_KEY=xxx QINIU_SECRET_KEY=yyy cargo test -p object-storage-qiniu -- --ignored --nocapture`
