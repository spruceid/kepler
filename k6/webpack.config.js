const path = require('path');

module.exports = {
    mode: 'production',
    entry: './main.js',
    output: {
        path: path.resolve(__dirname, 'dist'),
        libraryTarget: 'commonjs',
        filename: 'app.bundle.js',
    },
    module: {
        rules: [{ test: /\.js$/, use: 'babel-loader' }],
    },
    resolve: {
        fallback: {
            buffer: require.resolve('buffer/'),
            crypto: require.resolve('crypto-browserify'),
            events: require.resolve('events/'),
            http: require.resolve('stream-http'),
            https: require.resolve('https-browserify'),
            os: require.resolve('os-browserify'),
            path: require.resolve('path-browserify'),
            stream: require.resolve('stream-browserify'),
            url: require.resolve('url/'),
            util: require.resolve('util/'),
        }
    },
    target: 'web',
    externals: [/^(k6|https?\:\/\/)(\/.*)?/, {"fs": "commonjs fs"}],
    experiments: { asyncWebAssembly: true },
};
