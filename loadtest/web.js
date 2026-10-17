// k6 load test for the test-app web UI.
// SCENARIO=realistic | stress | login
import http from 'k6/http';
import { check, sleep } from 'k6';
import { Trend } from 'k6/metrics';

const BASE = __ENV.BASE || 'http://app:8080';
const EMAIL = 'load@example.org';
const PASSWORD = 'lasttest-passwort-1';
const WORDS = ['Einkauf', 'Projekt', 'Idee', 'Termin', 'Rezept', 'Homelab', 'Kino', 'Garten', 'gibtsnicht'];

const pageList = new Trend('page_notes_list', true);
const pageSearch = new Trend('page_notes_search', true);
const pageDetail = new Trend('page_note_detail', true);
const pageCreate = new Trend('page_note_create', true);
const pageLogin = new Trend('page_login', true);

const scenarios = {
  realistic: {
    executor: 'ramping-vus',
    startVUs: 0,
    stages: [
      { duration: '20s', target: 20 },
      { duration: '40s', target: 100 },
      { duration: '40s', target: 200 },
      { duration: '30s', target: 200 },
      { duration: '10s', target: 0 },
    ],
    gracefulRampDown: '5s',
    exec: 'browse',
    env: { THINK: '1' },
  },
  stress: {
    executor: 'constant-vus',
    vus: 50,
    duration: '45s',
    exec: 'browse',
    env: { THINK: '0' },
  },
  login: {
    executor: 'constant-arrival-rate',
    rate: Number(__ENV.RATE || 10),
    timeUnit: '1s',
    duration: '30s',
    preAllocatedVUs: 50,
    maxVUs: 400,
    exec: 'loginOnly',
  },
};

const name = __ENV.SCENARIO || 'realistic';
export const options = {
  // Keep the session cookie across iterations (k6 resets it by default).
  noCookiesReset: true,
  scenarios: { [name]: scenarios[name] },
  thresholds: {
    http_req_failed: ['rate<0.01'],
    http_req_duration: ['p(95)<500'],
  },
  summaryTrendStats: ['avg', 'med', 'p(90)', 'p(95)', 'p(99)', 'max'],
};

function login() {
  // No redirect following: timings must be those of the login POST itself.
  const res = http.post(`${BASE}/login`, { email: EMAIL, password: PASSWORD }, { tags: { page: 'login' }, redirects: 0 });
  pageLogin.add(res.timings.duration);
  check(res, {
    'login ok': (r) => r.status === 303 && r.headers['Location'] === '/notes' && !!r.cookies['tapp_session'],
  });
  return res;
}

let loggedIn = false;

export function browse() {
  if (!loggedIn) {
    login();
    loggedIn = true;
  }
  const think = __ENV.THINK === '1';

  const list = http.get(`${BASE}/notes`, { tags: { page: 'list' } });
  pageList.add(list.timings.duration);
  check(list, { 'list is notes page': (r) => r.status === 200 && !r.url.includes('/login') && r.body.includes('<h1>Notizen</h1>') });
  const ids = [...list.body.matchAll(/href="\/notes\/([0-9a-f-]{36})"/g)].map((m) => m[1]);
  const csrf = (list.body.match(/name="_csrf" value="([0-9a-f]+)"/) || [])[1];
  if (think) sleep(0.5 + Math.random());

  const word = WORDS[Math.floor(Math.random() * WORDS.length)];
  const search = http.get(`${BASE}/notes?q=${word}`, {
    headers: { 'HX-Request': 'true' },
    tags: { page: 'search' },
  });
  pageSearch.add(search.timings.duration);
  check(search, { 'search is notes page': (r) => r.status === 200 && !r.url.includes('/login') && r.body.includes('id="note-list"') });
  if (think) sleep(0.5 + Math.random());

  if (ids.length > 0) {
    const id = ids[Math.floor(Math.random() * ids.length)];
    const detail = http.get(`${BASE}/notes/${id}`, { tags: { page: 'detail' } });
    pageDetail.add(detail.timings.duration);
    check(detail, { 'detail is note page': (r) => r.status === 200 && r.body.includes('Alle Notizen') });
    if (think) sleep(1 + Math.random() * 2);
  }

  if (csrf && Math.random() < 0.1) {
    const created = http.post(
      `${BASE}/notes`,
      { _csrf: csrf, title: `Lasttest ${__VU}-${__ITER}`, body: 'Erstellt vom Lasttest' },
      { tags: { page: 'create' }, redirects: 0 },
    );
    pageCreate.add(created.timings.duration);
    check(created, { 'create ok': (r) => r.status === 303 && /^\/notes\/[0-9a-f-]{36}$/.test(r.headers['Location'] || '') });
    if (think) sleep(1);
  }
}

export function loginOnly() {
  http.cookieJar().clear(BASE);
  login();
}
