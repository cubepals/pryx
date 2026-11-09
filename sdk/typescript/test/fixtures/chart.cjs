// SPDX-License-Identifier: FSL-1.1-ALv2
/** The real chart renderer is Chart.js on an existing native canvas implementation. */
module.exports = function chart() {
  const { createCanvas } = require('@napi-rs/canvas')
  const { Chart } = require('chart.js/auto')
  const canvas = createCanvas(400, 250)
  const plot = new Chart(canvas, {
    type: 'line',
    data: { labels: ['a', 'b', 'c'], datasets: [{ label: 'values', data: [1, 3, 2] }] },
    options: { responsive: false, animation: false },
  })
  const png = canvas.toBuffer('image/png')
  plot.destroy()
  return png
}
