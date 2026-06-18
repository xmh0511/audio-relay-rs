// import http from 'k6/http';

// export const options = {
//   vus: 6,         // 6个并发
//   duration: '10s', // 测试10秒
// };

// const rates = [22050, 44100, 48000, 96000];

// export default function () {
//   const randomRate = rates[Math.floor(Math.random() * rates.length)];
//   const payload = JSON.stringify({ sample_rate: randomRate });
//   const params = {
//     headers: { 'Content-Type': 'application/json' },
//   };
//   http.post('http://127.0.0.1:8081/api/sample-rate', payload, params);
// }

// import http from 'k6/http';

// export const options = {
//   vus: 3000,        // 3000 个并发虚拟用户
//   iterations: 3000, // 总共执行 3000 次请求（每个 VU 执行 1 次）
// };

// const rates = [22050, 44100, 48000, 96000];

// export default function () {
//   const randomRate = rates[Math.floor(Math.random() * rates.length)];
//   const payload = JSON.stringify({ sample_rate: randomRate });
//   const params = {
//     headers: { 'Content-Type': 'application/json' },
//   };
//   http.post('http://127.0.0.1:8081/api/sample-rate', payload, params);
// }


import http from 'k6/http';
import exec from 'k6/execution';
export const options = {
  vus: 4,
  iterations: 4,
};

// const rates = [22050, 44100, 48000, 96000];

export default function () {
  const base_rate = 44100
  const currentIndex = exec.scenario.iterationInTest;
  const payload = JSON.stringify({ sample_rate: base_rate +(currentIndex+1)*100 });
  const params = {
    headers: { 'Content-Type': 'application/json' },
  };
  const res = http.post('http://127.0.0.1:8081/api/sample-rate', payload, params);
  console.log("post body: ", res.request.body);
}
