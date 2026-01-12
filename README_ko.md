# Rust + grammers Telegram 포워더

**Languages:** [English](README.md) | [日本語](README_ja.md) | 한국어

## 프로젝트 구조

```
rust_tdlib/
├── Cargo.toml              # 워크스페이스 설정
├── app/                    # 메인 엔트리
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs         # 메인 로직
│       └── thumb_dedup.rs  # 썸네일 중복 제거
├── core/                   # 핵심 로직
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── model.rs        # 데이터 모델
│       ├── pipeline.rs     # 처리 파이프라인
│       ├── filter.rs       # 키워드 필터
│       ├── replace.rs      # 텍스트 치환
│       ├── dedup.rs        # 다층 중복 제거
│       ├── album.rs        # 앨범 집계
│       ├── append.rs       # 헤더/푸터 추가
│       ├── dispatch.rs     # 워커 디스패처
│       ├── catch_up.rs     # 보정 처리
│       ├── hotreload.rs    # 핫 리로드
│       ├── media_filter.rs # 미디어 필터
│       ├── text_merge.rs   # 텍스트 병합
│       ├── entity_trim.rs  # 엔티티 트림
│       ├── throttle.rs     # FloodWait 제어
│       ├── thumb.rs        # pHash/dHash
│       └── sources.rs      # sources.txt 파싱
├── tdlib/                  # Telegram 어댑터 (grammers)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── client.rs       # 클라이언트 래퍼
│       ├── model.rs        # 모델 변환
│       └── send.rs         # 전송 처리
├── storage/                # 영속화
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── state.rs        # 상태 저장소
│       ├── dedup.rs        # 중복 저장소
│       ├── log.rs          # 로그 회전
│       └── session.rs      # WAL 정리
├── config/                 # 설정 로더/검증
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── loader.rs
│       ├── validate.rs
│       └── paths.rs
├── common/                 # 공통 유틸
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── utf16.rs
│       ├── time.rs
│       └── text.rs
├── logging/                # tracing 설정
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── sources.txt             # 소스 목록 (템플릿)
├── keywords.txt            # 키워드 필터 (옵션)
├── replacements.json       # 치환 규칙 (템플릿)
├── content_addition.json   # Header/Footer (템플릿)
├── logs.txt                # 런타임 로그
├── state.json              # 런타임 상태
├── deduplication.json      # 런타임 중복 DB
├── user_session*           # 런타임 세션 + WAL/SHM
└── data/                   # 썸네일 캐시
```

> 참고: `logs.txt`/`state.json`/`deduplication.json`/`user_session*`/`data/` 는 런타임에 생성되며 `.gitignore` 로 제외됩니다.

## 주요 기능

### 1. 설정
- 환경 변수 로드 (`.env` 지원)
- 설정 검증
- 경로 정규화
- Python 버전과 동일한 설정 항목
- 예시 설정 (`config.example.env`)

### 2. 영속화
- `state.json` 관리 (Format 1/Format 2)
- `deduplication.json` (version=2, LRU)
- 로그 회전
- 지연 저장
- Session WAL 체크포인트

### 3. 핵심 처리
- 키워드 필터 (화이트/블랙)
- 텍스트 치환 (정규식/정확 일치)
- 다층 중복 제거
  - SimHash
  - Jaccard (단문/장문)
  - 미디어 ID 시그니처
  - 미디어 메타 시그니처
  - pHash + dHash
  - 앨범 비율 기준
  - 단문+단일 미디어 이중 검증
- 헤더/푸터 추가 (UTF-16 제한)
- UTF-16 엔티티 오프셋 처리
- 긴 캡션 엔티티 트림
- 미디어 필터
- 앨범 전 텍스트 병합
- FloodWait 재시도

### 4. 앨범 집계
- album_id 기반 캐시
- 최대 개수 트리거 (기본 10)
- 지연 트리거
- 누락 보정
- 캡션 병합

### 5. Telegram 어댑터 (grammers-client)
- 초기화 (API_ID/API_HASH)
- 인증 (전화/코드/2FA)
- 자동 재연결
- 업데이트 구독 (Live)
- 메시지 가져오기
- 채팅 해석
- 텍스트/미디어/앨범 전송
- FloodWait 처리

### 6. 병렬 처리
- 비동기 큐
- 워커 풀 (기본 3)
- 그레이스풀 종료

### 7. 실행 모드
- **Live**: 실시간 업데이트 + 보정
- **Past**: 과거 폴링 + 앨범 보정

### 8. 핫 리로드
- 설정 파일 감시
- 자동 재로드
- 그레이스풀 재시작

### 9. 보정 처리
- 주기적 스캔
- 소스 단위 페이지 처리
- 앨범 고려 보정
- 갭 타임아웃

### 10. 로깅
- tracing 로그
- 레벨 제어 (DEBUG/INFO/WARNING/ERROR)
- `logs.txt` 로그 회전
- 콘솔 출력

## 알려진 한계

### 기능 강화
- 성능 지표 수집
- 재시도/백오프 개선
- 테스트 확장
- 통합/E2E 테스트

### 최적화
- 대용량 미디어 메모리 최적화
- 중복 DB 인덱싱
- 병렬 수 조정

### 호환성
- TDLib 네이티브 어댑터 (현재는 grammers)

## 의존성

```toml
[workspace.dependencies]
tokio = "1.35"
serde = "1.0"
serde_json = "1.0"
anyhow = "1.0"
thiserror = "1.0"
tracing = "0.1"
tracing-subscriber = "0.3"
tracing-appender = "0.2"
dotenv = "0.15"
regex = "1.10"
image = "0.25"

[workspace.dependencies.grammers-client]
version = "0.8"

[workspace.dependencies.grammers-session]
version = "0.8"

[workspace.dependencies.grammers-mtsender]
version = "0.8"

siphasher = "1.0"
```

## 설정

환경 변수 또는 `.env` 로 설정합니다.

- `TG_API_ID`: Telegram API ID
- `TG_API_HASH`: Telegram API Hash
- `TG_TARGET`: 대상 채팅
- `TG_SOURCE_FILE`: 소스 파일 경로 (기본 `sources.txt`)
- `TG_MODE`: 실행 모드 (`live`/`past`)
- `TG_PAST_LIMIT`: 과거 페이지 크기 (기본 2000)
- `TG_CATCH_UP_INTERVAL`: 보정 간격 초 (기본 300)
- `TG_ALBUM_DELAY`: 앨범 지연 (기본 5.0)
- `TG_ALBUM_MAX_ITEMS`: 최대 항목 수 (기본 10, 상한 10)
- `TG_ALBUM_BACKFILL_MAX_RANGE`: 보정 범위 (기본 20)
- `TG_FORWARD_DELAY`: 전송 지연 (기본 0)
- `TG_FLOOD_WAIT_MAX_RETRIES`: FloodWait 재시도 (기본 5)
- `TG_FLOOD_WAIT_MAX_TOTAL_WAIT`: FloodWait 총 대기 초 (기본 3600)
- `TG_REQUEST_TIMEOUT`: 요청 타임아웃 초 (기본 30)
- `TG_DISPATCHER_IDLE_TIMEOUT`: 디스패처 idle 초 (기본 300)
- `TG_UPDATES_IDLE_TIMEOUT`: 업데이트 idle 초 (기본 0 비활성)
- `TG_WORKER_COUNT`: 워커 수 (기본 3)
- `TG_ENABLE_FILE_FORWARD`: 파일 전송 허용 (기본 true)
- `TG_MEDIA_SIZE_LIMIT`: 미디어 크기 MB (기본 100)
- `TG_MEDIA_EXT_ALLOWLIST`: 확장자 허용 목록
- `TG_KEYWORD_FILE`: 키워드 파일 (설정 시만 활성)
- `TG_KEYWORD_CASE_SENSITIVE`: 키워드 대소문자 (기본 false)
- `TG_KEYWORD_RELOAD_INTERVAL`: 키워드 재로드 초 (기본 2)
- `TG_REPLACEMENT_FILE`: 치환 파일 (설정 시만 활성)
- `TG_CONTENT_ADDITION_FILE`: 추가 파일 (설정 시만 활성)
- `TG_DEDUP_FILE`: 중복 파일 경로 (기본 `deduplication.json`)
- `TG_DEDUP_LIMIT`: 텍스트 지문 한도 (기본 5000)
- `TG_MEDIA_DEDUP_LIMIT`: 미디어 지문 한도 (기본 15000)
- `TG_DEDUP_SIMHASH_THRESHOLD`: SimHash 임계값 (기본 3)
- `TG_DEDUP_JACCARD_SHORT_THRESHOLD`: Jaccard 단문 (기본 0.7)
- `TG_DEDUP_JACCARD_LONG_THRESHOLD`: Jaccard 장문 (기본 0.5)
- `TG_DEDUP_RECENT_TEXT_LIMIT`: 최근 텍스트 수 (기본 100)
- `TG_DEDUP_PHASH_THRESHOLD`: pHash 임계값 (기본 10)
- `TG_DEDUP_DHASH_THRESHOLD`: dHash 임계값 (기본 10)
- `TG_DEDUP_ALBUM_THUMB_RATIO`: 앨범 비율 (기본 0.34)
- `TG_DEDUP_SHORT_TEXT_THRESHOLD`: 단문 임계값 (기본 50)
- `TG_THUMB_DIR`: 썸네일 디렉터리 (기본 `data`)
- `TG_APPEND_LIMIT_WITH_MEDIA`: 미디어 추가 한도 (기본 1024)
- `TG_APPEND_LIMIT_TEXT`: 텍스트 추가 한도 (기본 4096)
- `TG_TEXT_MERGE_WINDOW`: 텍스트 병합 창 (기본 0)
- `TG_TEXT_MERGE_MIN_LEN`: 텍스트 최소 길이 (기본 0)
- `TG_TEXT_MERGE_MAX_ID_GAP`: ID 갭 (기본 5)
- `TG_STATE_FLUSH_INTERVAL`: 플러시 간격 초 (기본 5)
- `TG_SHUTDOWN_DRAIN_TIMEOUT`: 종료 대기 초 (기본 30)
- `TG_STATE_GAP_TIMEOUT`: 갭 타임아웃 초 (기본 300)
- `TG_STATE_PENDING_LIMIT`: Pending 한도 (기본 2000)
- `TG_HOTRELOAD_INTERVAL`: 핫 리로드 초 (기본 2)
- `TG_LOG_LEVEL`: 로그 레벨 (기본 INFO)
- `TG_LOG_MAX_LINES`: 로그 최대 줄 (기본 100)
- `TG_SESSION_NAME`: 세션 이름 (기본 `user_session`)
- `TG_TDLIB_DB_DIR`: TDLib DB 디렉터리 (옵션)
- `TG_TDLIB_FILES_DIR`: TDLib 파일 디렉터리 (옵션)
- `TG_DEVICE_MODEL`: 디바이스 모델 (옵션)
- `TG_SYSTEM_VERSION`: 시스템 버전 (옵션)
- `TG_APP_VERSION`: 앱 버전 (옵션)
- `TG_SYSTEM_LANG`: 시스템 언어 (옵션)
- `TG_USE_TEST_DC`: 테스트 DC 사용 (옵션)

## Python 호환성

### 호환 파일
- `sources.txt`
- `state.json` (Format 1/Format 2)
- `deduplication.json` (version=2)
- `keywords.txt`
- `replacements.json`
- `content_addition.json`

### 호환 설정
- 모든 환경 변수

### 호환 로직
- 키워드 필터
- 텍스트 치환
- 중복 판단 (SimHash/Jaccard/앨범 비율)
- 앨범 집계

## 빌드 및 실행

```bash
# 빌드
cargo build --release

# 설정 복사
cp config.example.env .env
# TG_API_ID / TG_API_HASH 입력
# sources.txt / replacements.json / content_addition.json / keywords.txt 필요 시 수정

# Live 모드
cargo run --release

# Past 모드
TG_MODE=past cargo run --release

# 빌드 검사
cargo check

# 포맷 검사
cargo fmt --all --check

# Clippy
cargo clippy --all-targets --all-features -- -D warnings

# 테스트
cargo test --workspace
```

## 로깅

로그는 `logs.txt` 에 기록됩니다. `TG_LOG_LEVEL` 로 레벨을 조정합니다.

## 성능 특징

- 비동기 병렬 처리 (Tokio)
- 워커 풀 전송
- 앨범 집계
- 중복 판단 우선순위: 미디어 ID → 썸네일 → 텍스트
- 이중 검증으로 오탐 감소
- 지연 저장으로 IO 감소

## 보안 참고

- `.env` 파일을 커밋하지 마세요
- API Hash 및 세션 파일 보호
- `state.json`과 `deduplication.json` 백업
